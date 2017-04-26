extern crate notify;
extern crate futures;
extern crate tokio_core;
extern crate tokio_tungstenite;
extern crate tungstenite;

use std::io::{Error, ErrorKind};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use notify::{RecommendedWatcher, Watcher, RecursiveMode};
use notify::DebouncedEvent;

use futures::Future;
use futures::stream::Stream;
use futures::Sink;

use tokio_core::reactor::Core;
use tokio_core::net::TcpListener;
use tokio_tungstenite::accept_async;
use tungstenite::Message;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::net::SocketAddr;
use futures::sync::mpsc::UnboundedSender;

fn main() {
	// Start websocket server
	let addr = "127.0.0.1:3000".parse().unwrap();

	let mut core = Core::new().unwrap();
	let handle = core.handle();
	let socket = TcpListener::bind(&addr, &handle).unwrap();
	println!("Listening on: {}", addr);

	let connections: Arc<Mutex<HashMap<SocketAddr, UnboundedSender<Message>>>> = Arc::new(Mutex::new(HashMap::new()));

	// Start file watch thread
	let (file_watch_tx, file_watch_rx) = channel();
	let file_connections_inner = connections.clone();
	let watch_thread = thread::spawn(move || {

		let mut watcher: RecommendedWatcher = Watcher::new(file_watch_tx, Duration::from_millis(100)).unwrap();

		let watch_path = "./";

		match watcher.watch(watch_path, RecursiveMode::Recursive) {
			Ok(_) => println!("Watching path {}", watch_path),
			Err(e) => println!("Got an error trying to watch files! {:?}", e)
		}

		loop {
			match file_watch_rx.recv() {
				Ok(event) => {
					match event {
						DebouncedEvent::Write(path) => {
							let path_string: String = path.to_str().unwrap().to_string();

							let mut conns = file_connections_inner.lock().unwrap();
							let iter = conns.iter_mut().map(|(_, v)| v);

							for tx in iter {
								tx.send(Message::Text(path_string.clone())).wait().unwrap();
							}
						}
						_ => {}
					}
				}
				Err(e) => println!("watch error: {:?}", e),
			}
		}
	});

	let server = socket.incoming().for_each(|(stream, addr)| {
		let connections_inner = connections.clone();

		let local_handle = handle.clone();

		// and_then -> Run after the chained future completes successfully, input is the unwrapped value from the chained future
		// then     -> Run after the chained future, input is a Result
		// or_else  -> Run after the chained future returns an error, input is the Error

		// All closure return values must implement IntoFuture

		accept_async(stream).and_then(move |ws_stream| {
			println!("{} connected", addr);

			let (tx, rx) = futures::sync::mpsc::unbounded();

			// In order to forward messages from rx to the sink, the Error type must be tungstenite::Error
			// rx will now be a Stream<Item = Message, Error = tungstenite::Error>
			let rx = rx.map_err(|_| tungstenite::Error::Io(Error::new(ErrorKind::Other, "Futures Channel RX Error")));

			connections_inner.lock().unwrap().insert(addr, tx);

			let (sink, stream) = ws_stream.split();

			// This stream will accept each message from the client and just print it. Type is Stream<Item = (), Error = tungstenite::Error>
			let ws_reader = stream
				.for_each(move |msg: Message| {
					println!("Got a message from {}: {:?}", addr, msg);
					Ok(())
				});

			// This stream simply forwards all messages from the channel (from the file watch thread) to the client
			let ws_writer = rx
				.forward(sink)
				.map(|_| ()); // Map to Stream<Item = (), Error = tungstenite::Error> so we can pass it to .select()

			let future_chain = ws_reader
				.select(ws_writer)
				.then(move |_| {
					connections_inner.lock().unwrap().remove(&addr);
					println!("{} disconnected", addr);
					Ok(())
				});

			local_handle.spawn(future_chain);

			Ok(())
		}).map_err(|e| {
			println!("Error during the websocket handshake occurred: {}", e);
			Error::new(ErrorKind::Other, e)
		})
	});

	core.run(server).unwrap();
	watch_thread.join().unwrap();
}
