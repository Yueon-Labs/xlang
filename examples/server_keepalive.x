module main

// Keepalive HTTP server: loops recv/send on each connection until the client
// closes (recv returns empty). Because xlang is blocking, it serves ONE
// connection at a time — concurrent keepalive connections starve. This is the
// workload that exposes the blocking-vs-epoll gap vs nginx.
fn main(): i32 {
    let fd: i32 = tcp_listen(28080)
    while true {
        let client: i32 = accept(fd)
        while true {
            let req: String = recv_str(client)
            if str_len(req) == 0 {
                break
            }
            send_str(client, "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
        }
        close_fd(client)
    }
    return 0
}
