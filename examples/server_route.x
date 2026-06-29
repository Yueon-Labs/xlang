module main

// Routing HTTP server: parses the request path and routes (nginx location-style)
// instead of returning one fixed response. GET /foo -> "bar", /bye -> "good",
// else -> "hello". Uses str_find (one of the string builtins added so xlang can
// actually inspect requests). Prefork 16 workers + keepalive.
fn handle(req: String): String {
    if str_find(req, "GET /foo ") >= 0 {
        return "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbar"
    }
    if str_find(req, "GET /bye ") >= 0 {
        return "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ngood"
    }
    return "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"
}

fn main(): i32 {
    let fd: i32 = tcp_listen(28080)
    let mut i: i32 = 0
    while i < 15 {
        let pid: i32 = fork()
        if pid == 0 {
            break
        }
        i += 1
    }
    while true {
        let client: i32 = accept(fd)
        while true {
            let req: String = recv_str(client)
            if str_len(req) == 0 {
                break
            }
            send_str(client, handle(req))
        }
        close_fd(client)
    }
    return 0
}
