module main

// epoll event-loop HTTP server (keepalive) — nginx's architecture. A SINGLE
// process multiplexes every connection through one epoll fd.
//   listen socket ready -> drain the accept queue (loop accept until EAGAIN),
//                          register each new client non-blocking.
//   client ready         -> recv request; empty => peer closed (del+close),
//                          else respond and keep the connection open.
// The accept-drain loop is the standard nginx pattern: one epoll_wait wakeup
// accepts ALL pending connections, instead of one-per-wakeup.
fn main(): i32 {
    let listen_fd: i32 = tcp_listen(28081)
    set_nonblock(listen_fd)
    epoll_create()
    epoll_add(listen_fd)
    while true {
        let fd: i32 = epoll_wait(-1)
        if fd == listen_fd {
            while true {
                let client: i32 = accept(listen_fd)
                if client < 0 {
                    break
                }
                set_nonblock(client)
                epoll_add(client)
            }
        } else {
            let req: String = recv_str(fd)
            if str_len(req) == 0 {
                epoll_del(fd)
                close_fd(fd)
            } else {
                send_str(fd, "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhello")
            }
        }
    }
    return 0
}
