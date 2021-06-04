use std::io::{ErrorKind, Result};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use mio::net::{SocketAddr, TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};

use super::LoadBalancer;

const SERVER_TOKEN: Token = Token(0);
const CLIENT_TOKEN: Token = Token(1);

pub struct Poller {
    poll: Arc<RwLock<Poll>>,
    events: Arc<RwLock<Events>>,
    balancer: LoadBalancer,
    should_cancel: Arc<RwLock<bool>>,
}

impl Poller {
    pub fn new(mut balancer: LoadBalancer) -> Self {
        let poll = Poll::new().unwrap();
        let events = Events::with_capacity(1024);
        let should_cancel = Arc::new(RwLock::new(false));

        let poll = Arc::new(RwLock::new(poll));
        balancer.register_poll(Arc::clone(&poll));
        balancer.start();

        Poller {
            poll,
            events: Arc::new(RwLock::new(events)),
            balancer,
            should_cancel,
        }
    }

    pub fn get_client_token() -> Token {
        CLIENT_TOKEN
    }

    pub fn initialize(&mut self) -> Result<()> {
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

        self.register_for_polling(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

        // START LISTENING
        println!("[Listener] Started listening on port {}", listening_port);
        loop {
            let mut events = self.events.write().unwrap();

            // POLL FOR EVENTS HERE
            match self.poll.write().unwrap().poll(&mut events, Some(Duration::from_millis(5))) {
                Ok(_) => {}
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                    *self.should_cancel.write().unwrap() = true;
                    self.balancer.stop();

                    println!("[Listener] Listening stopped");

                    // sleep a bit to allow all threads to exit gracefully
                    thread::sleep(Duration::from_millis(10));

                    break;
                }
                Err(e) => {
                    println!("Failed to poll for events! {}", e.to_string());
                    break;
                }
            };

            if events.is_empty() {
                continue;
            }

            // iterate through events
            let mut wake_up = false;
            for event in events.iter() {
                match event.token() {
                    SERVER_TOKEN => {
                        // listener accepted a new client
                        let mut connection = listener.accept()?;

                        self.register_for_polling(&mut connection.0, CLIENT_TOKEN, Interest::READABLE | Interest::WRITABLE)?;

                        self.balancer.add_client(connection.0);
                    }
                    CLIENT_TOKEN => {
                        // notify balancer of a change, wake it up
                        wake_up = true;
                    }
                    _ => {}
                }
            }

            if wake_up {
                self.balancer.wake_up();
            }
        }

        Ok(())
    }

    pub fn register_for_polling<S>(&self, stream: &mut S, token: Token, interest: Interest) -> Result<()>
    where
        S: mio::event::Source,
    {
        self.poll.read().unwrap().registry().register(stream, token, interest)?;
        Ok(())
    }
}
