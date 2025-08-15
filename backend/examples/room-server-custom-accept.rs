//! A chat server that broadcasts a message to all connections.
//!
//! This is a simple line-based server which accepts WebSocket connections,
//! reads lines from those connections, and broadcasts the lines to all other
//! connected clients.
//!
//! You can test this out by running:
//!
//!     cargo run --example room-server-custom-accept 127.0.0.1:12345
//!
//! And then in another window run:
//!
//!     cargo run --example client ws://127.0.0.1:12345/socket?name=test&transcribe_to=jp&translate_to=en
//!
//! You can run the second command in multiple windows and then chat between the
//! two, seeing the messages from the other client as they're received. For all
//! connected clients they'll all join the same room and see everyone else's
//! messages.

use hyper::{
    body::Incoming,
    header::{
        HeaderValue, CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION,
        UPGRADE,
    },
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};

use tokio_tungstenite::{
    tungstenite::{
        handshake::derive_accept_key,
        protocol::{Message, Role},
    },
    WebSocketStream,
};

type Tx = UnboundedSender<Message>;
type Body = http_body_util::Full<hyper::body::Bytes>;
use url::{form_urlencoded, Url};

struct PartialParticipant {
    name: String,
    transcribe_to: String,
    translate_to: String,
}

#[derive(Clone)]
struct Participant {
    name: String,
    _transcribe_to: String,
    _translate_to: String,
    sender: Tx,
}

type RoomName = String;

type RoomParticipants = HashMap<SocketAddr, Participant>;

type RoomMap = Arc<Mutex<HashMap<RoomName, RoomParticipants>>>;

fn get_room_participants(room_id: &str, room_map: &RoomMap) -> Vec<Participant> {
    let map = room_map.lock().unwrap();
    map.get(room_id).map(|peers| peers.values().map(|p| p.clone()).collect()).unwrap_or_default()
}

fn broadcast_ws_handshake_success(
    curr_addr: SocketAddr,
    curr_participant: &Participant,
    room_id: &str,
    room_map: &RoomMap,
) {
    let timestamp = chrono::Utc::now().to_rfc3339();

    // Send to owner
    let _ = curr_participant.sender.unbounded_send(Message::Text(
        json!({
            "type": "ws_handshake_status",
            "status": "connected",
            "timestamp": timestamp,
            "message": format!("You joined the room '{}'", room_id)
        })
        .to_string()
        .into(),
    ));

    // Collect senders for others without holding the lock
    let other_senders: Vec<Tx> =
        {
            let map = room_map.lock().unwrap();
            map.get(room_id)
                .map(|peers| {
                    peers
                        .iter()
                        .filter_map(|(peer_addr, p)| {
                            if *peer_addr != curr_addr {
                                Some(p.sender.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

    // Send to everyone else
    for tx in other_senders {
        let _ = tx.unbounded_send(Message::Text(
            json!({
                "type": "ws_handshake_status",
                "status": "connected",
                "timestamp": timestamp,
                "message": format!("{} joined the room", curr_participant.name)
            })
            .to_string()
            .into(),
        ));
    }
}

fn broadcast_ws_handshake_close(
    curr_addr: SocketAddr,
    curr_participant: &Participant,
    room_id: &str,
    room_map: &RoomMap,
) {
    let timestamp = chrono::Utc::now().to_rfc3339();

    // Send to owner
    let _ = curr_participant.sender.unbounded_send(Message::Text(
        json!({
            "type": "ws_handshake_status",
            "status": "close",
            "timestamp": timestamp,
            "message": format!("You left the room '{}'", room_id)
        })
        .to_string()
        .into(),
    ));

    // Collect senders for others without holding the lock
    let other_senders: Vec<Tx> =
        {
            let map = room_map.lock().unwrap();
            map.get(room_id)
                .map(|peers| {
                    peers
                        .iter()
                        .filter_map(|(peer_addr, p)| {
                            if *peer_addr != curr_addr {
                                Some(p.sender.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

    // Send to everyone else
    for tx in other_senders {
        let _ = tx.unbounded_send(Message::Text(
            json!({
                "type": "ws_handshake_status",
                "status": "close",
                "timestamp": timestamp,
                "message": format!("{} left the room", curr_participant.name)
            })
            .to_string()
            .into(),
        ));
    }
}

async fn handle_connection(
    room_id: String,
    room_map: RoomMap,
    partial_participant: PartialParticipant,
    ws_stream: WebSocketStream<TokioIo<Upgraded>>,
    addr: SocketAddr,
) {
    // ---- Create a sender channel for this participant ----
    let (tx, rx) = unbounded();

    // ---- Insert participant (safe now because name already validated) ----
    let participant = Participant {
        name: partial_participant.name,
        _transcribe_to: partial_participant.transcribe_to,
        _translate_to: partial_participant.translate_to,
        sender: tx,
    };

    let participant_for_broadcast = participant.clone();

    {
        let mut map = room_map.lock().unwrap();
        map.entry(room_id.clone()).or_default().insert(addr, participant);
        println!("WebSocket connection established: {}", addr);
    }
    // -- Broadcast WS Handshake
    broadcast_ws_handshake_success(addr, &participant_for_broadcast, &room_id, &room_map);

    // ---- Split into outgoing/incoming streams ----
    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        match msg {
            Message::Text(ref text) => {
                println!("[Room: {}] Received a message from {}: {}", room_id, addr, text);
            }
            Message::Binary(ref bin) => {
                println!("[Room: {}] Received binary from {}: {:?}", room_id, addr, bin);
            }
            _ => {}
        }

        let room_map = room_map.lock().unwrap();
        if let Some(peers) = room_map.get(&room_id) {
            for (peer_addr, participant) in peers.iter() {
                if *peer_addr != addr {
                    let _ = participant.sender.unbounded_send(msg.clone());
                }
            }
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);

    // -- Broadcast WS Handshake - Close
    broadcast_ws_handshake_close(addr, &participant_for_broadcast, &room_id, &room_map);

    // ---- Remove participant ----
    {
        let mut room_map = room_map.lock().unwrap();
        if let Some(peers) = room_map.get_mut(&room_id) {
            peers.remove(&addr);
        }
    }
}

async fn handle_request(
    room_map: RoomMap,
    mut req: Request<Incoming>,
    addr: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    let headers = req.headers();

    // Only accept proper WebSocket handshake requests
    if req.method() != Method::GET
        || headers.get(SEC_WEBSOCKET_VERSION).map(|h| h != "13").unwrap_or(true)
    {
        return Ok(Response::new(Body::from("Hi, you are in the wrong place.")));
    }

    println!("Received a new, potentially WS Handshake");
    println!("Request Path: {}", req.uri().path());

    // Extract room_id from path
    let mut room_id = String::from("default");
    let uri = req.uri().to_string();
    if let Ok(url) = Url::parse(&format!("ws://localhost{}", uri)) {
        let path_room = url.path().trim_start_matches('/');
        if !path_room.is_empty() {
            room_id = path_room.to_string();
        }
    }

    // Default participant data
    let mut participant_name = String::from("participant-name");
    let mut translate_to = String::from("en");
    let mut transcribe_to = String::from("jp");

    // Extract from query string
    if let Some(query_str) = req.uri().query() {
        let params: HashMap<_, _> =
            form_urlencoded::parse(query_str.as_bytes()).into_owned().collect();

        println!("Request Parameters: {:?}", params);

        if let Some(name) = params.get("name") {
            participant_name = name.clone();
        }
        if let Some(tt) = params.get("translate_to") {
            translate_to = tt.clone();
        }
        if let Some(tc) = params.get("transcribe_to") {
            transcribe_to = tc.clone();
        }
    }

    // Reject duplicate participant name
    {
        let rooms_lock = room_map.lock().unwrap();
        if let Some(room_participants) = rooms_lock.get(&room_id) {
            if room_participants.values().any(|p: &Participant| p.name == participant_name) {
                println!(
                    "Cannot upgrade or proceed. Participant {} is already in the room {}",
                    participant_name, room_id
                );
                let mut res = Response::new(Body::from(format!(
                    "Name '{}' is already in use",
                    participant_name
                )));
                *res.status_mut() = StatusCode::CONFLICT;
                return Ok(res);
            }
        }
    }

    println!(
        "Participant: {}, Translate to: {}, Transcribe to: {}",
        participant_name, translate_to, transcribe_to
    );

    println!("Request Headers:");
    for (ref header, _value) in req.headers() {
        println!("* {}", header);
    }

    let upgrade = HeaderValue::from_static("Upgrade");
    let websocket = HeaderValue::from_static("websocket");
    let key = headers.get(SEC_WEBSOCKET_KEY);
    let derived = key.map(|k| derive_accept_key(k.as_bytes()));
    let req_ver = req.version();

    // Upgrade the Connection
    tokio::task::spawn(async move {
        match hyper::upgrade::on(&mut req).await {
            Ok(upgraded) => {
                let upgraded = TokioIo::new(upgraded);

                let participant_obj = PartialParticipant {
                    name: participant_name,
                    transcribe_to: transcribe_to,
                    translate_to: translate_to,
                };

                handle_connection(
                    room_id,
                    room_map,
                    participant_obj,
                    WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await,
                    addr,
                )
                .await;
            }
            Err(e) => println!("upgrade error: {}", e),
        }
    });

    let mut res = Response::new(Body::default());

    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *res.version_mut() = req_ver;
    res.headers_mut().append(CONNECTION, upgrade);
    res.headers_mut().append(UPGRADE, websocket);
    res.headers_mut().append(SEC_WEBSOCKET_ACCEPT, derived.unwrap().parse().unwrap());
    // Let's add an additional header to our response to the client.
    res.headers_mut().append("MyCustomHeader", ":)".parse().unwrap());
    res.headers_mut().append("SOME_TUNGSTENITE_HEADER", "header_value".parse().unwrap());

    Ok(res)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let curr_room_state = RoomMap::new(Mutex::new(HashMap::new()));

    let addr =
        env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string()).parse::<SocketAddr>()?;

    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, remote_addr) = listener.accept().await?;
        let curr_room_state = curr_room_state.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service =
                service_fn(move |req| handle_request(curr_room_state.clone(), req, remote_addr));
            let conn = http1::Builder::new().serve_connection(io, service).with_upgrades();
            if let Err(err) = conn.await {
                eprintln!("failed to serve connection: {err:?}");
            }
        });
    }
}
