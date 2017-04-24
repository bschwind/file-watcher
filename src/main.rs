extern crate notify;
extern crate futures;
extern crate tokio_core;
extern crate tokio_tungstenite;
extern crate tungstenite;
extern crate multiqueue;

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

fn main() {
	// Start file watch thread
	let (file_watch_tx, file_watch_rx) = channel();
	let (file_watch_stream_tx, file_watch_stream_rx) = multiqueue::broadcast_fut_queue::<String>(10);
	// let file_watch_stream_rx = file_watch_stream_rx.wait().ok().unwrap();

	let mut local_file_watch_stream_tx = file_watch_stream_tx.clone();

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
							local_file_watch_stream_tx = local_file_watch_stream_tx.send(path_string).wait().unwrap();
						}
						_ => {}
					}
				}
				Err(e) => println!("watch error: {:?}", e),
			}
		}
	});

	// Start websocket server
	let addr = "127.0.0.1:3000".parse().unwrap();

	let mut core = Core::new().unwrap();
	let handle = core.handle();
	let socket = TcpListener::bind(&addr, &handle).unwrap();
	println!("Listening on: {}", addr);

	let server = socket.incoming().for_each(|(stream, addr)| {
		println!("{:?}", stream);
		println!("{:?}", addr);

		let local_handle = handle.clone();

		// and_then -> Run after the chained future completes successfully, input is the unwrapped value from the chained future
		// then     -> Run after the chained future, input is a Result
		// or_else  -> Run after the chained future returns an error, input is the Error

		// All closure return values must implement IntoFuture

		let local_file_rx = file_watch_stream_rx.add_stream();

		accept_async(stream).and_then(move |ws_stream| {
			println!("{} connected", addr);

			let (mut sink, stream) = ws_stream.split();

			let ws_reader = stream
				.for_each(move |msg: tungstenite::Message| {
					println!("Message: {:?}", msg);
					Ok(())
				}).map(|_| ()).map_err(|_| ());

			// let ws_writer = local_file_rx
			// 	.fold(sink, |mut sink, msg| {
			// 		sink.start_send(tungstenite::Message::Text(msg)).unwrap();
			// 		Ok(sink)
			// 	})
			// 	.map(|_| ())
			// 	.map_err(|_| ());

			let ws_writer = local_file_rx
				.for_each(move |msg| {
					sink.start_send(tungstenite::Message::Text(msg)).unwrap();
					Ok(())
				})
				.map(|_| ())
				.map_err(|_| ());

			let future_chain = ws_reader
				.select(ws_writer)
				.then(move |chain_result| {
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
}
