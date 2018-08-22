//! # Skynet
//!
//! Wrapped networking primitives for sending serialized structs over a connection.
//!
//! Currently contains the following:
//!
//! * `SerializedTcpStream`
//! * `SerializedUdpSocket`
//!
//! Also see [Middleman] for an alike project that works over [mio].
//!
//! [Middleman]: https://crates.io/crates/middleman
//! [mio]: [https://crates.io/crates/mio]

extern crate bincode;
extern crate serde;

use serde::*;

use std::io::{self, Read};
use std::net::*;

/// A wrapper over a `TcpStream` that sends and recieves serialized messages.
#[derive(Debug)]
pub struct SerializedTcpStream {
    buffer: Vec<u8>,
    stream: TcpStream
}

impl SerializedTcpStream {
    /// Wrap a `TcpStream` and set it to be nonblocking.
    ///
    /// See [`TcpStream::set_nonblocking`] for error types if the nonblocking call fails.
    ///
    /// Note: Depending ont the size of messages you are sending, it may be worth turning off [Nagle's algorithm] with [`TcpStream::set_nodelay`].
    /// This is generally recommended for games.
    ///
    /// [`TcpStream::set_nonblocking`]: https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_nonblocking
    /// [Nagle's algorithm]: https://en.wikipedia.org/wiki/Nagle%27s_algorithm
    /// [`TcpStream::set_nodelay`]: https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_nodelay
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;

        Ok(Self {
            stream,
            buffer: Vec::new()
        })
    }

    /// Get a reference to the inner stream.
    /// 
    /// Warning: Setting the stream to blocking can break `recv`.
    pub fn inner(&self) -> &TcpStream {
        &self.stream
    }

    /// Attempt to recieve a message off the stream.
    pub fn recv<R>(&mut self) -> bincode::Result<R>
        where for<'de> R: Deserialize<'de>
    {
        // Append new bytes onto the buffer (but don't propagate an error if there are no new bytes)
        if self.stream.read_to_end(&mut self.buffer).is_ok() {}
        
        // Get the size of the message
        let size = bincode::deserialize::<u64>(&self.buffer)? as usize;

        // If the buffer cant contain the size and the message then return without trying to serialize
        if self.buffer.len() < 8 + size {
            return Err(Box::new(bincode::ErrorKind::Io(io::ErrorKind::WouldBlock.into())));
        }

        // Get the message
        let message: R = bincode::deserialize(&self.buffer[8 .. 8 + size])?;
        // Crop the buffer to remove the used bytes
        self.buffer = self.buffer[8 + size ..].to_vec();
        Ok(message)
    }

    /// Attempt to send a message across the stream
    pub fn send<S: Serialize>(&self, data: &S) -> bincode::Result<()> {
        let size = bincode::serialized_size(data)?;
        bincode::serialize_into(&self.stream, &size)?;
        bincode::serialize_into(&self.stream, data)?;
        Ok(())
    }
}

/// A wrapper over a `UdpSocket` that sends and recieves serialized messages.
#[derive(Debug)]
pub struct SerializedUdpSocket {
    socket: UdpSocket
}

impl SerializedUdpSocket {
    /// Wrap a `UdpSocket` and set it to be nonblocking.
    ///
    /// See [`UdpSocket::set_nonblocking`] for error types if the nonblocking call fails.
    ///
    /// [`UdpSocket::set_nonblocking`]: https://doc.rust-lang.org/std/net/struct.UdpSocket.html#method.set_nonblocking
    pub fn new(socket: UdpSocket) -> io::Result<Self> {
        socket.set_nonblocking(true)?;

        Ok(Self {
            socket
        })
    }

    /// Get a reference to the inner socket.
    ///
    /// Warning: Setting the socket to blocking can break `recv_from`.
    pub fn inner(&self) -> &UdpSocket {
        &self.socket
    }

    /// Attempt to recieve a serialize a datagram from the socket, and return it and the address it came from.
    pub fn recv_from<R>(&self) -> bincode::Result<(R, SocketAddr)>
        where for<'de> R: Deserialize<'de>
    {
        let mut size = [0; 8];

        self.socket.peek(&mut size)?;
        let size = bincode::deserialize::<u64>(&size)? as usize;

        let mut buffer = vec![0; 8 + size + 1];

        self.socket.recv_from(&mut buffer)
            .map_err(|err| Box::new(bincode::ErrorKind::Io(err)))
            .and_then(|(read_size, addr)| {
                debug_assert_eq!(read_size, size + 8);
                Ok((bincode::deserialize(&buffer[8..8+size])?, addr))
            })
    }

    /// Attempt to serialize a message into a datagram and send it to an address.
    pub fn send_to<S: Serialize, A: ToSocketAddrs>(&self, data: &S, addr: A) -> bincode::Result<()> {
        let size = bincode::serialized_size(&data)?;
        let mut buffer = Vec::new();
        bincode::serialize_into(&mut buffer, &size)?;
        bincode::serialize_into(&mut buffer, data)?;
        
        debug_assert_eq!(buffer.len(), 8 + size as usize);
       
        self.socket.send_to(&buffer, addr)?;
        Ok(())
    }
}

mod tests {
    #![cfg(test)]

    use *;

    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_serialized_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let listener_addr = listener.local_addr().unwrap();

        let client = SerializedTcpStream::new(TcpStream::connect(listener_addr).unwrap()).unwrap();

        client.send(&Some(123_u8)).unwrap();

        sleep(Duration::from_millis(10));

        let stream = listener.incoming().next().unwrap().unwrap();
        let mut stream = SerializedTcpStream::new(stream).unwrap();

        let msg = stream.recv::<Option<u8>>().unwrap();

        assert_eq!(msg, Some(123_u8));
    }

    #[test]
    fn test_serialized_udp() {
        let socket_a = SerializedUdpSocket::new(UdpSocket::bind("127.0.0.1:0").unwrap()).unwrap();
        let socket_a_addr = socket_a.inner().local_addr().unwrap();

        let socket_b = SerializedUdpSocket::new(UdpSocket::bind("127.0.0.1:0").unwrap()).unwrap();
        let socket_b_addr = socket_b.inner().local_addr().unwrap();
        
        socket_b.send_to(&666_i32, socket_a_addr).unwrap();

        sleep(Duration::from_millis(10));

        let (msg, addr) = socket_a.recv_from::<i32>().unwrap();

        assert_eq!(msg, 666_i32);
        assert_eq!(addr, socket_b_addr);
    }

    #[test]
    fn test_u64_size() {
        // Double check the size of a serialized u64
        assert_eq!(bincode::serialized_size(&666_u64).unwrap(), 8);
    }
}