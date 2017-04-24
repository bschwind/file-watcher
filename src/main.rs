extern crate websocket;
extern crate notify;

#[macro_use]
extern crate chan;
extern crate bus;

use std::thread;
use websocket::{Server, Message};
use websocket::message::Type;
use websocket::result::WebSocketError;
use bus::Bus;

use notify::{RecommendedWatcher, Watcher, RecursiveMode};
use notify::DebouncedEvent;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
	let server = Server::bind("127.0.0.1:3000").unwrap();

	let (file_watch_tx, file_watch_rx) = channel();
	let mut file_watch_bus = Arc::new(Mutex::new(Bus::new(10)));
	let local_bus = file_watch_bus.clone();

	let watch_thread = thread::spawn(move || {

		let mut watcher: RecommendedWatcher = Watcher::new(file_watch_tx, Duration::from_millis(10)).unwrap();

		let watch_path = "YOUR_WATCH_PATH_HERE";

		match watcher.watch(watch_path, RecursiveMode::Recursive) {
			Ok(_) => println!("Watching path {}", watch_path),
			Err(e) => println!("Got an error trying to watch files! {:?}", e)
		}

		loop {
			match file_watch_rx.recv() {
				Ok(event) => {
					match event {
						DebouncedEvent::Write(path) => {
							println!("Path write: {}", path.display());

							let path_string: String = path.to_str().unwrap().to_string();

							local_bus.lock().unwrap().broadcast(path_string);
						}
						_ => println!("Something else")
					}
				}
				Err(e) => println!("watch error: {:?}", e),
			}
		}
	});

	for request in server.filter_map(Result::ok) {
		let file_rx_local = file_watch_bus.lock().unwrap().add_rx();

		thread::spawn(move || {
			let mut client = request.accept().unwrap();

			let ip = client.peer_addr().unwrap();

			println!("Connection from {}", ip);

			let message: Message = Message::text("Welcome".to_string());
			client.send_message(&message).unwrap();

			let (mut receiver, mut sender) = client.split().unwrap();

			let (ws_tx, ws_rx) = chan::sync(0);
			let (file_tx, file_rx) = chan::sync(0);

			let client_recv_thread = thread::spawn(move || {
				for message in receiver.incoming_messages() {
					ws_tx.send(message);
				}
			});

			let file_recv_thread = thread::spawn(move || {
				for r in file_rx_local.into_iter() {
					file_tx.send(r);
				}
			});

			loop {
				chan_select! {
					default => thread::sleep(Duration::from_millis(1000)),
					file_rx.recv() -> path => {
						let message: Message = Message::text(path.unwrap());
						sender.send_message(&message).unwrap();
					},
					ws_rx.recv() -> msg => {
						let msg: Option<Result<Message, WebSocketError>> = msg;

						match msg {
							Some(Ok(m)) => {
								match m.opcode {
									Type::Close => {
										let msg = Message::close();
										let _ = sender.send_message(&msg);
										println!("Client {} disconnected", ip);
										return;
									}
									Type::Ping => {
										let msg = Message::pong(m.payload);
										let _ = sender.send_message(&msg);
									}
									_ => {
										
									}
								}
							}
							Some(Err(e)) => {
								println!("Error: {:?}", e);
								let msg = Message::close();
								let _ = sender.send_message(&msg);
								println!("Client {} disconnected", ip);
								return;
							}
							None => {
								println!("No message received");
								let msg = Message::close();
								sender.send_message(&msg);
							}
						}

						let message: Message = Message::text("thanks for the message".to_string());
						sender.send_message(&message);
					}
				}
			}
		});
	}
}
