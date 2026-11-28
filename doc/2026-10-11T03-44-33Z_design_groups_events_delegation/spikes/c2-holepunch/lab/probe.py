"""NAT behaviour probe for the lab topology. Run with `python3 -I`.

  probe.py PORT SECONDS [--delay S] [--to IP:PORT ...]

Binds UDP PORT. After DELAY seconds, sends one datagram to each --to destination, then prints
every datagram received for SECONDS. Each receive line shows the source the receiver saw, which
is how a NAT's external mapping is read.
"""

import argparse
import socket
import time


def main():
    p = argparse.ArgumentParser()
    p.add_argument("port", type=int)
    p.add_argument("seconds", type=float)
    p.add_argument("--delay", type=float, default=0.0)
    p.add_argument("--to", action="append", default=[])
    args = p.parse_args()

    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("0.0.0.0", args.port))
    s.settimeout(0.2)
    time.sleep(args.delay)
    for dest in args.to:
        host, dport = dest.rsplit(":", 1)
        s.sendto(f"from-{args.port}".encode(), (host, int(dport)))
        print(f"sent local={args.port} to={dest}", flush=True)
    end = time.time() + args.seconds
    while time.time() < end:
        try:
            data, (ip, sport) = s.recvfrom(2048)
        except socket.timeout:
            continue
        print(f"recv local={args.port} from={ip}:{sport} data={data.decode(errors='replace')}", flush=True)


if __name__ == "__main__":
    main()
