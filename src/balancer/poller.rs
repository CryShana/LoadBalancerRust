use std::io::{ErrorKind, Result};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use mio::net::{TcpListener};
use mio::{Events, Interest, Poll, Token};

use super::LoadBalancer;

pub struct Poller {
    balancer: LoadBalancer,
    should_cancel: Arc<RwLock<bool>>,
}

impl Poller {
    pub fn new(mut balancer: LoadBalancer) -> Self {
        let should_cancel = Arc::new(RwLock::new(false));
        balancer.start();

        let mut p = Poller {
            balancer,
            should_cancel,
        };

        p.initialize().unwrap();

        p
    }

    fn initialize(&mut self) -> Result<()> {
        // prepare the ctrl+c handler for graceful stop
        let cancel = Arc::clone(&self.should_cancel);
        ctrlc::set_handler(move || {
            *cancel.write().unwrap() = true;
        })
        .expect("Failed to set Ctrl+C handler!");

        Ok(())
    }

    pub fn start_listening(&mut self, listening_port: i32) -> Result<()> {
        let addr = format!("0.0.0.0:{}", listening_port).parse().unwrap();
        let mut listener = TcpListener::bind(addr)?;

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(512);
        poll.registry().register(&mut listener, Token(0), Interest::READABLE)?;

        // START LISTENING
        println!("[Listener] Started listening on port {}", listening_port);
        loop {
            if *self.should_cancel.read().unwrap() {
                self.balancer.stop();
                println!("[Listener] Listening stopped");

                // sleep a bit to allow all threads to exit gracefully
                thread::sleep(Duration::from_millis(10));
                break;
            }

            // poll for events here (with timeout to check of [should_cancel])
            match poll.poll(&mut events, Some(Duration::from_millis(5))) {
                Ok(_) => {}
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                    // this handler does not get called on Windows, so we use timeout and check it outside
                    *self.should_cancel.write().unwrap() = true;  
                }
                Err(e) => {
                    println!("Failed to poll for events! {}", e.to_string());
                    break;
                }
            };

            if events.is_empty() {
                continue;
            }

            for event in events.iter() {
                match event.token() {
                    _ => {
                        // listener accepted a new client
                        let connection = listener.accept()?;
                        self.balancer.add_client(connection.0);
                    }
                }
            }
        }

        Ok(())
    }
}
