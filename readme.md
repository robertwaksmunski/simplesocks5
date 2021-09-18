# simplesocks5

simplesocks5 is meant to be a simple, elegant and readable RFC1928 compliant socks5 implementation in pure, safe Rust without any dependencies or crates beyond the standard library. Something that can be left listening on a public interface without a worry of a machine security compromise. Author has very extensive experience with async/await and epoll/kqueue based event loops (Tokyo, mio and async_std), but for a project this simple he wishes to explore the behaviour of worker threads and blocking system calls on modern Unix like machines. Implementation is integration tested with Curl and Firefox.

### Architecture

Main thread binds to a socket and spawns a client connection handling thread on accept witch negotiates the protocol and authentication. Request is parsed and remote connection is opened. Data read from the client stream is then piped to the remote stream. Data going in the other direction; from the remote stream to client stream is piped by a small spawned lambda thread. Since we are using blocking read system calls we need 2 threads, one for each data direction. Any connection error or EOF on either side of the pipe will cause the other end to be closed as well. Once connections are closed the worker threads terminate.

### TODO

- Config struct and argument parsing
- Username/Password authentication RFC1929
- UDP Clients
