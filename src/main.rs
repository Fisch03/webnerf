use axum::{
    Router,
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::any,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::mpsc};
use tower_http::services::ServeDir;

#[derive(Debug, Clone)]
struct AppState {
    open_connections: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Command>>>>,
}

#[derive(Debug, Deserialize, Serialize)]
enum Command {
    EstablishConnection(String),
    Fire,
}

#[tokio::main]
async fn main() {
    colog::init();

    let app = Router::new()
        .route("/api/receive", any(receive_handler))
        .route("/api/send", any(send_handler))
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .with_state(AppState {
            open_connections: Arc::new(Mutex::new(HashMap::new())),
        });

    let listener = TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to address");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Failed to start server");
}

async fn obtain_pwd(ws: &mut WebSocket, socket_addr: SocketAddr) -> Option<String> {
    let Some(Ok(Message::Text(initial_message))) = ws.recv().await else {
        log::error!("Receiver {:?} didnt connect properly", socket_addr);
        return None;
    };

    let Ok(Command::EstablishConnection(pwd)) = serde_json::from_str::<Command>(&initial_message)
    else {
        log::error!("Receiver {:?} sent invalid initial message", socket_addr);
        return None;
    };

    Some(pwd)
}

async fn receive_handler(
    ws: WebSocketUpgrade,
    socket_addr: ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let Some(pwd) = obtain_pwd(&mut socket, socket_addr.0).await else {
            return;
        };

        let (sender, mut receiver) = mpsc::unbounded_channel();
        {
            let mut open_connections = state.open_connections.lock().unwrap();
            open_connections.insert(pwd.clone(), sender);
        }

        log::info!(
            "Receiver {:?} connected with password: {}",
            socket_addr,
            pwd
        );

        loop {
            tokio::select! {
                Some(cmd) = receiver.recv() => {
                    let cmd = serde_json::to_string(&cmd).expect("Failed to serialize command");
                    if socket.send(Message::Text(cmd.into())).await.is_err() {
                        log::error!("Failed to send message to {:?}", socket_addr);
                    }
                }
                Some(Ok(_)) = socket.recv() => {}
                else => {
                    log::info!("Receiver {:?} disconnected", socket_addr);
                    break;
                }
            }
        }

        let mut open_connections = state.open_connections.lock().unwrap();
        open_connections.remove(&pwd);
    })
}

async fn send_handler(
    ws: WebSocketUpgrade,
    socket_addr: ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let mut pwd = String::new();

        log::info!("Sender {:?} connected", socket_addr);

        loop {
            tokio::select! {
                Some(Ok(Message::Text(data))) = socket.recv() => {
                    match serde_json::from_str::<Command>(&data) {
                        Ok(Command::Fire) => {
                            let open_connections = state.open_connections.lock().unwrap();
                            if let Some(sender) = open_connections.get(&pwd) {
                                sender.send(Command::Fire).unwrap_or_else(|e| {
                                    log::error!("Failed to send command to receiver: {}", e);
                                });
                            } else {
                                log::warn!("No receiver found for password: {}", pwd);
                            }
                        },
                        Ok(Command::EstablishConnection(new_pwd)) => pwd = new_pwd,

                        Err(_) => {
                            log::error!("Sender {:?} sent invalid command: {}", socket_addr, data);
                            continue;
                        },
                    }

                }
                else => {
                    log::info!("Sender {:?} disconnected", socket_addr);
                    break;
                }
            }
        }
    })
}
