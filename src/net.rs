//! Small blocking line-oriented TCP transport shared by the protocol drivers.
//! Both PJLink and Panasonic NTCONTROL terminate messages with a carriage return.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const IO_TIMEOUT: Duration = Duration::from_secs(4);

pub struct LineConn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl LineConn {
    pub fn connect(host: &str, port: u16) -> io::Result<Self> {
        Self::connect_with(host, port, CONNECT_TIMEOUT, IO_TIMEOUT)
    }

    pub fn connect_with(
        host: &str,
        port: u16,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> io::Result<Self> {
        let addrs: Vec<SocketAddr> = (host.trim(), port).to_socket_addrs()?.collect();
        let mut last = io::Error::new(io::ErrorKind::NotFound, "host did not resolve");
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, connect_timeout) {
                Ok(stream) => return Self::from_stream(stream, io_timeout),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub fn from_stream(stream: TcpStream, io_timeout: Duration) -> io::Result<Self> {
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(io_timeout))?;
        stream.set_write_timeout(Some(io_timeout))?;
        Ok(Self { stream, buf: Vec::new() })
    }

    pub fn send(&mut self, text: &str) -> io::Result<()> {
        self.stream.write_all(text.as_bytes())
    }

    /// Read one CR-terminated line (without the terminator). Blank lines are skipped.
    pub fn read_line(&mut self) -> io::Result<String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\r') {
                let raw: Vec<u8> = self.buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&raw[..raw.len() - 1])
                    .trim_matches(|c| c == '\n' || c == '\0' || c == ' ')
                    .to_string();
                if line.is_empty() {
                    continue;
                }
                return Ok(line);
            }
            let mut tmp = [0u8; 512];
            let n = self.stream.read(&mut tmp)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed by projector"));
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > 16 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "response too long"));
            }
        }
    }
}

/// Human-friendly description of a socket error.
pub fn describe(e: &io::Error) -> String {
    use io::ErrorKind::*;
    match e.kind() {
        TimedOut | WouldBlock => "No response (timed out)".into(),
        ConnectionRefused => "Connection refused (is network control enabled on the projector?)".into(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => "Connection dropped".into(),
        NotFound => "Host name did not resolve".into(),
        _ => {
            let s = e.to_string();
            if s.contains("No route") || s.contains("unreachable") {
                "Network unreachable".into()
            } else {
                s
            }
        }
    }
}
