# Client code injection

## Bug fixing

This project was originally made to
fix [a bug in the client](https://unix.stackexchange.com/questions/447898/why-does-a-program-with-fork-sometimes-print-its-output-multiple-times).

The printf buffer should get flushed before `fork`ing so it does not get copied between child processes hence preventing
duplicate log lines when outputting anywhere other than the console. The objective was then to process those log lines
by piping the client output to an external program.

The best solution was found to be injecting a dynamic library hooking into the runtime libc relocations. Our
own `fork()` function would call `fflush(stdout)` before the real `fork`.

```c
pid_t fork(void) {
  fflush(stdout);
  return __fork();
}
```

Here, `__fork()` is a glibc (or standard libc ?) symbol that points to the same `fork()` function we know. This allows
us to call the original function back.

## Injecting our code

To make the client use our function we can simply force the dynamic loader to load our `.so` before glibc.

```shell
LD_PRELOAD=path/to/so ./client1 ...
```

Yes, on Linux this is really as simple as it looks.

## Trace processing

Since we already are injecting our code into the client, why not simply put the log processing there and forget about
piping its output ? We can make our own `printf` and gather logs in memory with a timestamp then upload all data on
exit. Thing is, we would need to handle lists, serialization and network in C which is a no-no. So a Rust library does
the heavy lifting behind the scenes.

## Why not all Rust then ?

C variadics are only available in nightly Rust. They are required for `printf`.
