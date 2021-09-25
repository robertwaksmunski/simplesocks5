# simplesocks5

simplesocks5 is meant to be a simple, small, elegant and readable RFC1928 partially compliant socks5 implementation in pure, safe Rust without any dependencies or crates beyond the standard library. Something that can be left listening on a public interface without a worry of a machine security compromise. Author has very extensive experience with async/await and epoll/kqueue based event loops (Tokyo, mio and async_std), but for a project this simple he wishes to explore the behaviour of worker threads and blocking system calls on modern Unix like machines. Implementation is integration tested with Curl and Firefox.

### Usage

```
simplesocks5 0.0.0.0:1080
```
```
simplesocks5 -v \[::1\]:1080
```

Where 0.0.0.0:1080 or [::1]:1080 is the bind IP and port.
You may add -v -vv -vvv for various levels of debug logging.

### Architecture

Main thread binds to a socket and spawns a client connection handling thread on accept witch negotiates the protocol and authentication. Request is parsed and remote connection is opened. Data read from the client stream is then piped to the remote stream. Data going in the other direction; from the remote stream to client stream is piped by a small spawned lambda thread. Since we are using blocking read system calls we need 2 threads, one for each data direction. Any connection error or EOF on either side of the pipe will cause the other end to be closed as well. Once connections are closed the worker threads terminate.

### Overhead

Here is a dtrace instrumentation on a FreeBSD 13.0 machine of a 1GB curl file download over https.

```
curl -x socks5://127.0.0.1:1080 --output /dev/null https://proof.ovh.net/files/1Gb.dat
```

```
dtrace -n 'syscall:::entry /execname == "simplesocks5"/ { @[probefunc] = count(); }'
dtrace: description 'syscall:::entry ' matched 1142 probes
^C

  _umtx_op                                                          1
  accept4                                                           1
  connect                                                           1
  getsockname                                                       1
  socket                                                            1
  cpuset_getaffinity                                                2
  munmap                                                            2
  setsockopt                                                        2
  shutdown                                                          2
  sigfastblock                                                      2
  thr_exit                                                          2
  thr_new                                                           2
  fcntl                                                             3
  madvise                                                           4
  mmap                                                              4
  mprotect                                                          4
  close                                                             5
  sigaltstack                                                       6
  sendto                                                       486733
  recvfrom                                                     486735
```

Not great, not terrible. With default kernel configuration of 1500 threads per process we can proxy about 750 connections at a time.

```
sysctl kern.threads.max_threads_per_proc
kern.threads.max_threads_per_proc: 1500
```

### Benchmark

Direct curl download:

```
curl --output /dev/null http://37.58.58.140/speedtest/1000mb.bin
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100  953M  100  953M    0     0  62.8M      0  0:00:15  0:00:15 --:--:-- 70.2M
```

Proxied download (simplesocks5):

```
curl -x socks5://127.0.0.1:1080 --output /dev/null http://37.58.58.140/speedtest/1000mb.bin
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100  953M  100  953M    0     0  49.2M      0  0:00:19  0:00:19 --:--:-- 42.1M
```

Proxied download (dante-1.4.3):

```
curl -x socks5://127.0.0.1:1081 --output /dev/null http://37.58.58.140/speedtest/1000mb.bin
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100  953M  100  953M    0     0  42.9M      0  0:00:22  0:00:22 --:--:-- 41.0M
```

From Leaseweb DE to Finland, 20ms away. 

Not the most stable connection, so like with most benchmarks please take it with a ton of salt. At first glance we see about 20% overhead but still better than market leader.

### TODO

- Username/Password authentication RFC1929
- UDP Clients
- Tests / Fuzzer