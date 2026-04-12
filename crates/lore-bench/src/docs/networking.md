# Networking

The library exposes low-level TCP and UDP networking primitives for use cases
that go beyond the built-in HTTP client.

## TCP Client

Open a TCP socket connection and send raw bytes:

```rust
use mylib::net::TcpStream;

let mut stream = TcpStream::connect("127.0.0.1:9000").await?;
stream.write_all(b"PING\r\n").await?;

let mut buf = [0u8; 64];
let n = stream.read(&mut buf).await?;
println!("received: {}", std::str::from_utf8(&buf[..n])?);
```

## TCP Server

Accept incoming TCP socket connections:

```rust
use mylib::net::TcpListener;

let listener = TcpListener::bind("0.0.0.0:9000").await?;
loop {
    let (stream, addr) = listener.accept().await?;
    tokio::spawn(handle_client(stream, addr));
}
```

## UDP

Send and receive UDP datagrams:

```rust
use mylib::net::UdpSocket;

let socket = UdpSocket::bind("0.0.0.0:0").await?;
socket.send_to(b"hello", "127.0.0.1:9001").await?;

let mut buf = vec![0u8; 1024];
let (n, from) = socket.recv_from(&mut buf).await?;
```

## DNS Resolution

Resolve a hostname to its IP addresses:

```rust
let addrs = mylib::net::dns::resolve("example.com").await?;
for addr in addrs {
    println!("{addr}");
}
```

## Unix Domain Sockets

On Unix systems, connect over a local socket for lower overhead:

```rust
let stream = mylib::net::UnixStream::connect("/run/myapp.sock").await?;
```

## Connection Timeouts

All connection attempts respect the global timeout configuration.  Override
per-connection:

```rust
TcpStream::connect_timeout("remote:9000", Duration::from_secs(5)).await?;
```
