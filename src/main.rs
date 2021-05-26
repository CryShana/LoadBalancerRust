use std::fmt::format;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::io::{Result, prelude::*};
use std::process::exit;
use std::str::FromStr;
use std::time::Duration;
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
    
    // IDEA: save new clients to a pre-allocated array OR list --- multiple threads are then spawned and handle clients in this array (because it's non-blocking, can jsut go through many of them)
    // - Need a way to remove inactive clients --> timeouts on no receive or sending data?
    
    /*
    ctrlc::set_handler(|| {
        println!("Signal received");
        

    }).expect("Failed to set Ctrl+C handler!");
    */

    // get endpoint
    let listening_port = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            println!("Please specify local port to listen on!");
            exit(1);
        }
    };
    
    let pool = ThreadPool::new(4);
    let addr = format!("0.0.0.0:{}", listening_port);

    let listener: TcpListener = TcpListener::bind(addr).expect("Failed to bind to port!");
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
    println!("Connected from {}", addr.to_string());

    let target_port: u16 = 5000;
    let target_addr: IpAddr = IpAddr::from_str("127.0.0.1").unwrap();
    let target_socket = SocketAddr::new(target_addr, target_port);
    let target = format!("{}:{}", target_addr, target_port);

    // establish connection to target
    let mut str = match TcpStream::connect_timeout(&target_socket, Duration::from_millis(1000)) {
        Ok(stream) => stream,
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
            // timed out
            println!("[{} <-> {}] Timed out connection to server", addr, target);
            return;
        }
        Err(err) => {
            println!("[{} <-> {}] Error while trying to connect to server: {}", addr, target, err.to_string());
            return;
        }
    };

    // make sure it's nonblocking
    str.set_nonblocking(true).unwrap();
    stream.set_nonblocking(true).unwrap();

    let mut buffer = [0; 4096];
    loop {
        // READ FROM CLIENT
        let read: i32 = match stream.read(&mut buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(err) => {
                // error with connection
                println!("[{} <-> {}] Connection to client failed!", addr, target);
                break;
            }
        };

   
        // WRITE TO SERVER
        if read > 0 {
            str.write(&buffer[..(read as usize)]).unwrap();
        }
        else if read == 0 {
            println!("[{} <-> {}] Zero buffer from client", addr, target);
            break;
        }

        // READ FROM SERVER
        let reads: i32 = match str.read(&mut buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(err) => {
                // error with connection
                println!("[{} <-> {}] Connection to server failed!", addr, target);
                break;
            }
        };

        // WRITE TO CLIENT
        if reads > 0 {
            stream.write(&buffer[..(reads as usize)]).unwrap();
        }
        else if reads == 0 {
            println!("[{} <-> {}] Zero buffer from server", addr, target);
            break;
        }
    }

    println!("[{} <-> {}] Connection ended", addr, target);
}