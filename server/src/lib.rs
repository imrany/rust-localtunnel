// create the endpoint, proxy.threethain.dev/did-123, proxy.threethain.xyz?new
// create a new clent manager, the manager should listen on the assigned port
// send request to the custom domain, get client id
// get the client manager with client id
// client manager handle the request.

use std::{collections::HashMap, sync::{Mutex, Arc}, net::SocketAddr, io};

use actix_web::{get, web, App, HttpServer, Responder, HttpResponse, dev::ConnectionInfo};
use hyper::{upgrade::Upgraded, service::service_fn, server::conn::http1};
use serde::{Serialize, Deserialize};
use tokio::{net::{TcpListener, TcpStream}};
use tldextract::{TldExtractor, TldOption};

use axum::{
    body::{self, Body},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    extract::{ws::{WebSocketUpgrade, WebSocket, Message}, TypedHeader,}, headers,
};

struct State {
    manager: Arc<Mutex<ClientManager>>,
}

#[get("/hello/{name}")]
async fn greet(name: web::Path<String>) -> impl Responder {
    format!("Hello {name}!")
}

#[get("/api/status")]
async fn status() -> impl Responder {
    let status = ApiStatus {
        tunnels_count: 10,
        tunels: "kaichao".to_string(),
    };

    HttpResponse::Ok().json(status)
}

#[get("/{endpoint}")]
async fn request_endpoint(endpoint: web::Path<String>, state: web::Data<State>) -> impl Responder {
    log::info!("Request proxy endpoint, {}", endpoint);
    let mut manager = state.manager.lock().unwrap();
    log::info!("get lock, {}", endpoint);
    manager.put(endpoint.to_string()).await.unwrap();

    let info = ProxyInfo {
        id: endpoint.to_string(),
        port: manager.clients.get(&endpoint.to_string()).unwrap().lock().unwrap().port.unwrap(),
        max_conn_count: 10,
        url: format!("{}.localhost", endpoint.to_string()),

    };

    log::info!("proxy info, {:?}", info);
    HttpResponse::Ok().json(info)
}

// TODO use tokio tcplistener directly, no need for authentiacation, since it's from public user requests
#[get("/")]
async fn request(conn: ConnectionInfo, state: web::Data<State>) -> impl Responder {
    let host = conn.host();

    let tld: TldExtractor = TldOption::default().build();
    if let Ok(uri) = tld.extract(host) {
        if let Some(endpoint) = uri.subdomain {
            log::info!("uri, {:?}", endpoint);
        }
    } else {
        log::info!("error");
    }
    format!("hello {host}")
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiStatus {
    tunnels_count: u16,
    tunels: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProxyInfo {
    id: String,
    port: u16,
    max_conn_count: u8,
    url: String,
}

struct ClientManager {
    clients: HashMap<String, Arc<Mutex<Client>>>,
    tunnels: u16,
}

impl ClientManager {
    pub fn new() -> Self {
        ClientManager {
            clients: HashMap::new(),
            tunnels: 0,
        }
    }

    pub async fn put(&mut self, url: String) -> io::Result<()> {
        if self.clients.get(&url).is_none() {
            let client = Arc::new(Mutex::new(Client::new()));
        
            self.clients.insert(url, client.clone() );

            let mut client = client.lock().unwrap();
            client.listen().await;
            
        }

        Ok(())
    }
}

struct Client {
    available_sockets: Arc<Mutex<Vec<TcpStream>>>,
    port: Option<u16>,
}

impl Client {
    pub fn new() -> Self {
        Client {
            available_sockets: Arc::new(Mutex::new(vec![])),
            port: None,
        }
    }
    pub async fn listen(&mut self) -> io::Result<()> {
        // TODO port should > 1000
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr().unwrap().port();
        self.port = Some(port);

        let sockets = self.available_sockets.clone();

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, addr)) => {
                        println!("new client connection: {:?}", addr);
                        let mut sockets = sockets.lock().unwrap();
                        sockets.push(socket)
                    },
                    Err(e) => println!("Couldn't get client: {:?}", e),
                }
            }
        });

        Ok(())
    }

    pub fn take(&mut self) -> Option<TcpStream> {
        let mut sockets = self.available_sockets.lock().unwrap();
        sockets.pop()
    }
}

// TODO proxy_port, port -> admin_port
// require_auth: bool
// start a tcplistener on proxy port
pub async fn create(domain: String, port: u16, secure: bool, max_sockets: u8) {
    log::info!("Create proxy server at {} {} {} {}", &domain, port, secure,  max_sockets);

    let manager = Arc::new(Mutex::new(ClientManager::new()));
    let state = web::Data::new(State {
        manager: manager.clone(),
    });

    // tokio::spawn(async move {
    //     let router = Router::new().route("/", routing::get(|| async { "Hello, World!" }));

    //     let service = tower::service_fn(move |req: Request<Body>| {
    //         let router = router.clone();
    //         async move {
    //             if req.method() == Method::CONNECT {
    //                 proxy(req).await
    //             } else {
    //                 router.oneshot(req).await.map_err(|err| match err {})
    //             }
    //         }
    //     });

    //     let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
        
    //     axum::Server::bind(&addr)
    //         .http1_preserve_header_case(true)
    //         .http1_title_case_headers(true)
    //         .serve(Shared::new(service))
    //         .await
    //         .unwrap();
    // });

    // tokio::spawn(async move {
    //     let app = Router::new()
    //         // routes are matched from bottom to top, so we have to put `nest` at the
    //         // top since it matches all routes
    //         .route("/ws", routing::get(ws_handler));
    //         let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
    //         log::info!("listening on {}", addr);
    //         axum::Server::bind(&addr)
    //             .serve(app.into_make_service())
    //             .await
    //             .unwrap();
    // });

    tokio::spawn(async move {
        let addr: SocketAddr = ([127, 0, 0, 1], 3001).into();
        log::info!("listening on {}", addr);
        let listener = TcpListener::bind(addr).await.unwrap();

        loop {
            let (stream, _) = listener.accept().await.unwrap();

            log::info!("Accept proxy request");

            // This is the `Service` that will handle the connection.
            // `service_fn` is a helper to convert a function that
            // returns a Response into a `Serive`.
            let manager = manager.clone();
            let service = service_fn(move |req| {
                println!("uri ========= {}", req.uri());
                println!("host ========= {:?}", req.headers());
                let hostname = req.headers().get("host").unwrap().to_str().unwrap();
                println!("hostname ========= {}", hostname);

                let endpoint = extract(hostname.to_string());
                let mut manager = manager.lock().unwrap();
                let mut client = manager.clients.get_mut(&endpoint).unwrap().lock().unwrap();
                let client_stream = (*client).take().unwrap();

                async move {
                    let (mut sender, conn) = hyper::client::conn::http1::handshake(client_stream).await.unwrap();
                    tokio::spawn(async move {
                        if let Err(err) = conn.await {
                            log::error!("Connection failed: {:?}", err);
                        }
                    });

                    sender.send_request(req).await
                }
            });

            tokio::spawn(async move {
                if let Err(err) = http1::Builder::new()
                    .serve_connection(stream, service)
                    .await
                {
                    log::error!("Failed to serve connection: {:?}", err);
                }
            });
        }
    });

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(greet)
            .service(status)
            .service(request_endpoint)
            .service(request)
    })
    .bind(("127.0.0.1", port)).unwrap()
    .run()
    .await
    .unwrap();
}

fn extract(hostname: String) -> String {
    // TODO regex
    let hostname = hostname
        .replace("http://", "")
        .replace("https://", "")
        .replace("ws", "")
        .replace("wss", "");

    hostname.split(".").next().unwrap().to_string()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> impl IntoResponse {
    if let Some(TypedHeader(user_agent)) = user_agent {
        println!("`{}` connected", user_agent.as_str());
    }

    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    if let Some(msg) = socket.recv().await {
        if let Ok(msg) = msg {
            match msg {
                Message::Text(t) => {
                    println!("client sent str: {:?}", t);
                }
                Message::Binary(_) => {
                    println!("client sent binary data");
                }
                Message::Ping(_) => {
                    println!("socket ping");
                }
                Message::Pong(_) => {
                    println!("socket pong");
                }
                Message::Close(_) => {
                    println!("client disconnected");
                    return;
                }
            }
        } else {
            println!("client disconnected");
            return;
        }
    }

    loop {
        if socket
            .send(Message::Text(String::from("Hi!")))
            .await
            .is_err()
        {
            println!("client disconnected");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

async fn proxy(req: Request<Body>) -> Result<Response, hyper::Error> {
    log::info!("Request: {:?}", req);

    if let Some(host_addr) = req.uri().authority().map(|auth| auth.to_string()) {
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    if let Err(e) = tunnel(upgraded, host_addr).await {
                        log::warn!("server io error: {}", e);
                    };
                }
                Err(e) => log::warn!("upgrade error: {}", e),
            }
        });

        Ok(Response::new(body::boxed(body::Empty::new())))
    } else {
        log::warn!("CONNECT host is not socket addr: {:?}", req.uri());
        Ok((
            StatusCode::BAD_REQUEST,
            "CONNECT must be to a socket address",
        )
            .into_response())
    }
}

async fn tunnel(mut upgraded: Upgraded, addr: String) -> std::io::Result<()> {
    let mut server = TcpStream::connect(addr).await?;

    let (from_client, from_server) =
        tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    log::info!(
        "client wrote {} bytes and received {} bytes",
        from_client,
        from_server
    );

    Ok(())
}
