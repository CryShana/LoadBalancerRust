use std::fmt::format;
use std::net::{TcpListener, TcpStream};
use std::io::{Result, prelude::*};
use std::process::exit;
use std::{env, thread};
use threadpool::ThreadPool;

mod balancer;
use balancer::LoadBalancer;

fn main() -> Result<()> {
    // - file that contains list of hosts in format [IP]:[Port]
    // - load balancing algorithm is also given as an argument - default is round robin
    // - use threadpool to handle clients

    // algorithm
    // - handle clients by IP / or by connection / or by time period?
    // - clients are assigned hosts as they connect and stay with those hosts (assigned host is determined by the method used)
    
    
    /*
    ctrlc::set_handler(|| {
        println!("Signal received");
        

    }).expect("Failed to set Ctrl+C handler!");
    */

    // get endpoint
    let local_address = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            println!("Please specify local IP endpoint to listen on!");
            exit(1);
        }
    };
    
    let pool = ThreadPool::new(4);

    let listener: TcpListener = TcpListener::bind(local_address).expect("Failed to bind to port!");
    listener.set_nonblocking(true).expect("Failed to put listener into non-blocking mode!");
    
    // accept connections and process them serially
    for stream in listener.incoming() {
        match stream {
            Ok(str) => {
                // handle client using the threadpool
                pool.execute(|| {
                    handle_client(str);
                });
            },
            // because we are not blocking (to exit gracefully), we need to ignore non-blocking errors
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                continue;
            },
            // handle actual errors here
            Err(err) => {
                println!("Failed to accept connection! {}", err.to_string());
            }
        }
    }

    Ok(())
}

fn handle_client(mut stream: TcpStream) {
    let addr = stream.peer_addr().unwrap();

    let mut buffer = [0; 1024];
 
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                continue;
            },
            Err(err) => {
                // error with connection
                break;
            }
        };
    
        if read > 0 {
            // handle content
            println!("From client {} got following request:\n{}", addr.to_string(), String::from_utf8_lossy(&buffer[..read]));
        }
    }

    println!("Lost connection to {}", addr.to_string());

    // get host to connect to

    // connect to that host - upon FAILURE, try next one

    // send buffer to that host
}