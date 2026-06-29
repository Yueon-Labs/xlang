#!/usr/bin/env python3
"""Keepalive load generator: N processes, each holds ONE persistent connection
and pipelines requests over it (no TCP handshake per request). This stresses
concurrent-connection handling — where a blocking server (one connection at a
time) starves the rest while nginx's epoll serves them all.
Usage: python3 bench_ka.py <host> <port> <seconds> <procs>"""
import http.client
import multiprocessing
import sys
import time


def worker(host: str, port: int, seconds: float, q) -> None:
    deadline = time.time() + seconds

    def new_conn():
        return http.client.HTTPConnection(host, port, timeout=5)

    conn = new_conn()
    ok = 0
    while time.time() < deadline:
        try:
            conn.request("GET", "/")
            resp = conn.getresponse()
            resp.read()
            ok += resp.status == 200
        except Exception:
            try:
                conn.close()
            except Exception:
                pass
            conn = new_conn()
    try:
        conn.close()
    except Exception:
        pass
    q.put(ok)


def main() -> None:
    host, port, seconds, procs = sys.argv[1], int(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4])
    mgr = multiprocessing.Manager()
    q = mgr.Queue()
    jobs = [multiprocessing.Process(target=worker, args=(host, port, seconds, q)) for _ in range(procs)]
    for j in jobs:
        j.start()
    for j in jobs:
        j.join()
    total = sum(q.get() for _ in range(procs))
    print(f"{total / seconds:.0f} req/s  ({total} ok in {seconds}s @ {procs} keepalive conns)")


if __name__ == "__main__":
    main()
