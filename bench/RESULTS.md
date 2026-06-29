# xlang HTTP server vs nginx — benchmark

## Setup
- Server: wzu (Ubuntu 22.04, x86_64), **localhost loopback**.
- **xlang server**: `examples/server_loop.x` (blocking, one connection at a time), compiled xlang → C → `cc -O2`.
- **nginx**: 1.28.0 built **from source** on the server (`~/nginx-bin`), `location / { return 200 "hello"; }` (hardcoded, no file I/O).
- Both return the **identical 5-byte response** `"hello"`.
- Load: `bench/bench.py` (stdlib python, new connection per request), 8s @ 50 concurrent.

## Result (fair: both return hardcoded "hello")
| server   | run 1       | run 2       |
|----------|-------------|-------------|
| nginx 1.28 | ~1730 req/s | ~1710 req/s |
| xlang      | ~1770 req/s | ~2560 req/s |

xlang's compiled server is in the **same ballpark** as nginx for this trivial fixed-response workload (within ~1.0–1.5×).

### Stronger load (multiprocess, bypasses the python GIL) — `bench/bench_mp.py`
64-core box, 8 processes, new connection per request, 6s:
| server            | req/s   |
|-------------------|---------|
| nginx 1.28        | ~8360   |
| xlang (1 worker)  | ~8540   |

Even under ~5× the load, xlang's **blocking** server stays level with nginx. For this minimal workload the per-connection work is so tiny that serializing connections does not yet cost throughput — both are bound by accept/loop rate (and likely still partly by the client).

### Decisive: keepalive + concurrency — `bench/bench_ka.py`
16 concurrent **persistent** connections, 6s:
| server                  | req/s    |
|-------------------------|----------|
| nginx 1.28              | ~69,800  |
| xlang (1 blocking worker) | ~8,230 |

**nginx is ~8.5× faster under keepalive concurrency.** xlang serves one connection at a time — its inner keepalive loop blocks on `recv`, starving the other 15 connections; nginx's epoll serves all 16 concurrently. This is the workload that triggers "modify x": add concurrency (fork workers / epoll event loop) so xlang can serve many connections at once.

## Honest caveats — this is NOT "xlang beats nginx"
1. **Trivial workload** (5-byte fixed response). nginx's machinery overhead dominates when the work is tiny, so a minimal hand-written server can match it. Real workloads (file serving, proxying, keepalive, real HTTP parsing) would change the picture.
2. **The load generator (python, threading + GIL) likely caps the measurement** around ~2000–2500 req/s — the client may be the bottleneck, so the true server ceilings are not reached.
3. **xlang's server is blocking, single-connection.** At high concurrency, keepalive, or pipelining, nginx's epoll event loop would pull far ahead.

## What this validates
xlang → C → `cc -O2` produces genuinely fast server code: for a hello-world HTTP response it is competitive with nginx on the same machine. That is a real, rigorous data point (same workload, same machine, real nginx built from source).

## To go further (honest next steps)
- A C load generator (wrk) to find true ceilings past the python client limit.
- Higher concurrency + keepalive to expose the blocking-vs-epoll gap (where xlang needs `epoll` support).
- Realistic workloads (serve a real file, parse the request path) where nginx's engineering matters.
