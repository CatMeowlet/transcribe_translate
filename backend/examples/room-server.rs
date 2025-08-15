//! A chat server with rooms and participant list broadcasting
//!
//! Usage:
//! Server: cargo run --example server_room 127.0.0.1:12345
//! Client: cargo run --example client ws://127.0.0.1:12345/room?name=John

use std::{
    collections::HashMap,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt, SinkExt, StreamExt};
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        protocol::Message,
    },
    WebSocketStream,
};
use tungstenite::handshake::server::ErrorResponse;
use url::Url;

type Tx = UnboundedSender<Message>;

#[derive(Clone)]
struct Participant {
    name: String,
    sender: Tx,
}

type RoomName = String;

type RoomParticipants = HashMap<SocketAddr, Participant>;

type RoomMap = Arc<Mutex<HashMap<RoomName, RoomParticipants>>>;

/// Collect senders for a room without holding the lock while sending
fn collect_room_senders(rooms: &RoomMap, room_id: &str) -> Vec<Tx> {
    let map = rooms.lock().unwrap();
    map.get(room_id)
        .map(|peers| peers.values().map(|p| p.sender.clone()).collect())
        .unwrap_or_default()
}

/// Broadcast participant count (lock-free sending)
fn broadcast_count(rooms: &RoomMap, room_id: &str) {
    let senders = collect_room_senders(rooms, room_id);
    let count = senders.len();

    let msg = json!({
        "type": "count",
        "count": count
    })
    .to_string();

    for tx in senders {
        let _ = tx.unbounded_send(Message::Text(msg.clone().into()));
    }
}

/// Broadcast participant list (lock-free sending)
fn broadcast_participants(rooms: &RoomMap, room_id: &str) {
    let (list, senders): (Vec<String>, Vec<Tx>) = {
        let map = rooms.lock().unwrap();
        if let Some(peers) = map.get(room_id) {
            let list: Vec<String> = peers.values().map(|p| p.name.clone()).collect();
            let senders: Vec<Tx> = peers.values().map(|p| p.sender.clone()).collect();
            (list, senders)
        } else {
            (Vec::new(), Vec::new())
        }
    };

    let msg = json!({
        "type": "participants",
        "participants": list
    })
    .to_string();

    for tx in senders {
        let _ = tx.unbounded_send(Message::Text(msg.clone().into()));
    }
}

/// Handle all incoming messages from this client and broadcast them to others
fn handle_incoming(rooms: &RoomMap, room_id: &str, addr: SocketAddr, msg: Message) {
    let senders: Vec<Tx> = {
        let map = rooms.lock().unwrap();
        if let Some(peers) = map.get(room_id) {
            peers
                .iter()
                .filter(|(peer_addr, _)| *peer_addr != &addr) // exclude self
                .map(|(_, p)| p.sender.clone())
                .collect()
        } else {
            Vec::new()
        }
    };

    for tx in senders {
        let _ = tx.unbounded_send(msg.clone());
    }
}

/// Forward messages from other participants to this client
async fn read_received<S>(rx: S, outgoing: impl SinkExt<Message> + Unpin)
where
    S: futures_util::Stream<Item = Message> + Unpin,
{
    let _ = rx.map(Ok).forward(outgoing).await;
}

fn process_header_and_validate_participant_name(
    request: &Request,
    rooms: &RoomMap,
) -> Result<(String, String), ErrorResponse> {
    let mut room_id = String::from("default");
    let mut display_name = String::from("Anonymous");

    let uri = request.uri().to_string();
    if let Ok(url) = Url::parse(&format!("ws://localhost{}", uri)) {
        room_id = url.path().trim_start_matches('/').to_string();

        if room_id.is_empty() {
            room_id = "default".into();
        }

        if let Some(name) = url.query_pairs().find(|(k, _)| k == "name") {
            display_name = name.1.to_string();
        }
    }

    // Check if name already exists in room
    {
        let rooms_lock = rooms.lock().unwrap();
        if let Some(participants) = rooms_lock.get(&room_id) {
            if participants.values().any(|p| p.name == display_name) {
                // Fail handshake with HTTP 409 and reason
                let resp = Response::builder()
                    .status(409)
                    .body(Some(format!("Name '{}' is already in use", display_name)))
                    .unwrap();
                return Err(resp);
            }
        }
    }

    Ok((room_id, display_name))
}

async fn handle_connection(rooms: RoomMap, stream: TcpStream, connection_addr: SocketAddr) {
    let mut room_id = String::new();
    let mut display_name = String::new();

    // ---- WebSocket handshake & extract room/name ----
    let ws_stream = accept_hdr_async(stream, |req: &Request, resp: Response| {
        match process_header_and_validate_participant_name(req, &rooms) {
            Ok((rid, dname)) => {
                room_id = rid;
                display_name = dname;
                Ok(resp)
            }
            Err(reject_resp) => Err(reject_resp), // reject handshake here
        }
    })
    .await;

    let ws_stream: WebSocketStream<TcpStream> = match ws_stream {
        Ok(stream) => {
            println!("{} joined room '{}' as '{}'", connection_addr, room_id, display_name);
            stream
        }
        Err(tungstenite::Error::Http(response)) => {
            // Extract and log reason from rejection
            if let Some(reason) = response.body() {
                println!(
                    "Rejected connection from {}: {}",
                    connection_addr,
                    String::from_utf8_lossy(&reason)
                );
            } else {
                println!(
                    "Rejected connection from {} with status {}",
                    connection_addr,
                    response.status()
                );
            }
            return;
        }
        Err(e) => {
            println!("Handshake error from {}: {:?}", connection_addr, e);
            return;
        }
    };

    // ---- Create a sender channel for this participant ----
    let (tx, rx) = unbounded();

    // ---- Insert participant (safe now because name already validated) ----
    {
        let mut map = rooms.lock().unwrap();
        map.entry(room_id.clone())
            .or_default()
            .insert(connection_addr, Participant { name: display_name.clone(), sender: tx });

        println!("=== Current Room State ===");
        for (room, participants) in map.iter() {
            println!("Room: {}", room);
            for (addr, participant) in participants.iter() {
                println!("  Addr: {:?}, Name: {}", addr, participant.name);
            }
        }
        println!("==========================");
    }

    // ---- Broadcast updated room state ----
    broadcast_count(&rooms, &room_id);
    broadcast_participants(&rooms, &room_id);

    // ---- Split into outgoing/incoming streams ----
    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        handle_incoming(&rooms, &room_id, connection_addr, msg);
        future::ok(())
    });
    let receive_from_others = read_received(rx, outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} left room '{}'", connection_addr, room_id);

    // ---- Remove participant ----
    {
        let mut room_map = rooms.lock().unwrap();
        if let Some(peers) = room_map.get_mut(&room_id) {
            peers.remove(&connection_addr);
        }
    }

    broadcast_count(&rooms, &room_id);
    broadcast_participants(&rooms, &room_id);
}

#[tokio::main]
async fn main() -> Result<(), IoError> {
    let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let listener = TcpListener::bind(&addr).await.expect("Can't bind");

    // Init Room to Empty
    let rooms: RoomMap = Arc::new(Mutex::new(HashMap::new()));

    println!("Listening on {}", addr);

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(rooms.clone(), stream, addr));
    }

    Ok(())
}
