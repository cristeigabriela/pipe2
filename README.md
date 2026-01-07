# pipe2

Rust implementation of correct reading of `stdout`/`stderr` while a program is running.

## Benefits

This allows you to silmultaneously capture the `stdout` and `stderr` contents, separately, and allows you to relay them as soon as there is new available data on the pipes, by writing the contents directly to your buffer.

## Scenarios

This is useful in scenarios where you want your child program to act as if it's inherited it's parents' I/O handles, but then you also need to read the contents of the operation(s).

## Challenges

In scenarios like this, `impl Read` for `ChildStdout`/`ChildStderr` on Windows is not helpful. For that, we need to obtain the raw handles created for `stdout` and `stderr` for the child, and use the non-blocking API [PeekNamedPipe](https://learn.microsoft.com/sl-si/windows/win32/api/namedpipeapi/nf-namedpipeapi-peeknamedpipe) to check if there are new bytes available to be read from the specified handle:
> "The function always returns immediately in a single-threaded application, even if there is no data in the pipe. The wait mode of a named pipe handle (blocking or nonblocking) has no effect on the function."

Moreover, for a full understanding of other issues that can appear here, you also need to refer to the [CreateNamedPipe](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea) API to understand the nature of the buffers that will replace our pipes for the program execution.
> "Whenever a pipe write operation occurs, the system first tries to charge the memory against the pipe write quota. If the remaining pipe write quota is enough to fulfill the request, the write operation completes immediately. If the remaining pipe write quota is too small to fulfill the request, the system will try to expand the buffers to accommodate the data using nonpaged pool reserved for the process. The write operation will block until the data is read from the pipe so that the additional buffer quota can be released. Therefore, if your specified buffer size is too small, the system will grow the buffer as needed, but the downside is that the operation will block. If the operation is overlapped, a system thread is blocked; otherwise, the application thread is blocked."
TL;DR: it is our responsibility to make sure that, during program execution, we read the contents actively being written to the pipes, to prevent the child process in turn from ending up in a blocking I/O operation.

## Solution

Pipe2, using the aforementioned API, works in a loop where it waits for when the pipe has new data available, and only then proceeds to call to the `impl Read` operations. The loop runs until `child.try_wait()` returns an exit status.

## Why not just use the blocking API?

Unix: without making `stdout` and `stderr` non-blocking, the operation will only complete on application exit.
> **It *does* read `stdout` correctly, but it does *not* allow us to see the contents *as* the program is executing.**

Windows: going into `impl Read` without explicitly having data available on the pipe will end up in the operation acting as blocking until application exit.
> **Same as above.**

## When is this even useful?

Example: reading (and saving) the output of an application like [kubelogin](https://github.com/Azure/kubelogin), where the user-facing content comes through `stderr`, and the token is returned through `stdout`. An application trying to present this to the user is meant to only present the `stderr`, and internally process the `stdout`.

## Why not just use inherit?

At least on Windows, you can't mark that a IO pipe be inherited, then read from, for what should be obvious reasons when stated as such.
