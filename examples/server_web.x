module main

// server_web <docroot> — epoll event-loop HTTP FILE server (nginx's real job).
// Parses each request line (GET /path HTTP/1.1), maps /path -> docroot/path
// ("/" -> index.html), serves the file with Content-Length/Type + 200, or 404.
// Connection closed after each response. Built on the epoll builtins; tests
// xlang's string parsing (str_find/str_slice) and file I/O (read_file etc.).

fn mime_of(path: String): String {
    if str_find(path, ".html") >= 0 {
        return "text/html"
    }
    if str_find(path, ".htm") >= 0 {
        return "text/html"
    }
    if str_find(path, ".css") >= 0 {
        return "text/css"
    }
    if str_find(path, ".js") >= 0 {
        return "application/javascript"
    }
    if str_find(path, ".json") >= 0 {
        return "application/json"
    }
    if str_find(path, ".txt") >= 0 {
        return "text/plain"
    }
    return "application/octet-stream"
}

fn parse_path(req: String): String {
    let sp1: i32 = str_find(req, " ")
    if sp1 < 0 {
        return "/index.html"
    }
    let rest: String = str_slice(req, sp1 + 1, str_len(req))
    let sp2: i32 = str_find(rest, " ")
    let mut path: String = ""
    if sp2 < 0 {
        path = rest
    } else {
        path = str_slice(rest, 0, sp2)
    }
    if str_eq(path, "/") {
        path = "/index.html"
    }
    return path
}

fn serve(fd: i32, full: String, path: String): i32 {
    if file_exists(full) {
        let size: i32 = file_size(full)
        let mime: String = mime_of(path)
        sb_new()
        sb_push("HTTP/1.1 200 OK\r\n")
        sb_push("Content-Type: ")
        sb_push(mime)
        sb_push("\r\nContent-Length: ")
        sb_push(int_to_str(size))
        sb_push("\r\nConnection: keep-alive\r\n\r\n")
        send_str(fd, sb_str())
        let ffd: i32 = open_read(full)
        sendfile_fd(fd, ffd, size)
        close_fd(ffd)
    } else {
        send_str(fd, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n")
    }
    return 0
}

fn main(): i32 {
    let mut docroot: String = "webroot"
    if argc() >= 2 {
        docroot = argv(1)
    }
    let listen_fd: i32 = tcp_listen(28082)
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
                // Non-blocking client: fast event loop. Large file bodies go out
                // via sendfile_fd which retries on EAGAIN (completes in full), so
                // non-blocking no longer truncates responses. TCP_NODELAY avoids
                // the Nagle + delayed-ACK 40ms stall (headers + sendfile body are
                // two sends; without NODELAY the second waits for the first's ACK).
                set_nonblock(client)
                set_nodelay(client)
                epoll_add(client)
            }
        } else {
            let req: String = recv_str(fd)
            if str_len(req) == 0 {
                epoll_del(fd)
                close_fd(fd)
            } else {
                let path: String = parse_path(req)
                let full: String = str_concat(docroot, path)
                serve(fd, full, path)
            }
        }
    }
    return 0
}
