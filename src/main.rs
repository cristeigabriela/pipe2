use std::io::{self, Write};
use std::process::{Command, Stdio, exit};
use std::time::Duration;

#[cfg(unix)]
use nix::fcntl::{FcntlArg, OFlag, fcntl};
#[cfg(unix)]
use std::io::Read;

#[cfg(windows)]
mod windows_pipe_utils {
    use std::io;
    use std::os::windows::io::AsRawHandle;

    use winapi::shared::winerror::{ERROR_BROKEN_PIPE, ERROR_SUCCESS};
    use winapi::um::errhandlingapi::{GetLastError, SetLastError};
    use winapi::um::fileapi::ReadFile;
    use winapi::um::namedpipeapi::PeekNamedPipe;

    pub fn can_read<R: AsRawHandle>(pipe: &R) -> io::Result<bool> {
        let handle = pipe.as_raw_handle();
        let mut bytes_avail = 0u32;
        let ok = unsafe {
            PeekNamedPipe(
                handle as _,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut bytes_avail,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            // NOTE(gabriela): it's... fine.
            unsafe {
                if GetLastError() == ERROR_BROKEN_PIPE {
                    SetLastError(ERROR_SUCCESS);
                }
            }
        }
        Ok(bytes_avail > 0)
    }

    pub fn read_pipe<R: AsRawHandle>(pipe: &mut R, buf: &mut [u8]) -> io::Result<usize> {
        let handle = pipe.as_raw_handle();
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle as _,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(read as usize)
    }
}

fn main() -> io::Result<()> {
    let mut child = Command::new("ping")
        .args(if cfg!(windows) {
            &["-n", "10", "localhost"]
        } else {
            &["-c", "10", "localhost"]
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().expect("Failed to capture stdout");
    let mut stderr = child.stderr.take().expect("Failed to capture stderr");

    #[cfg(unix)]
    {
        fcntl(&stdout, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        fcntl(&stderr, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
    }

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut scratchpad = vec![0u8; 1024];

    // NOTE(gabriela): pipes are read during program execution, ensuring that no issues such as the pipe buffer
    // becoming full and leading to blocking on process I/O operations happens.
    //
    // On Windows: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    //
    // "Whenever a pipe write operation occurs, the system first tries to charge the memory against the pipe write quota.
    // If the remaining pipe write quota is enough to fulfill the request, the write operation completes immediately.
    // If the remaining pipe write quota is too small to fulfill the request, the system will try to expand the buffers
    // to accommodate the data using nonpaged pool reserved for the process. The write operation will block until the data
    // is read from the pipe so that the additional buffer quota can be released."
    //
    // TL;DR: the `stdout`/`stderr` pipe buffers could get filled up if we don't read them *as* the process is executing,
    // causing blocks on I/O.
    let Some(exit) = loop {
        #[cfg(unix)]
        {
            match stdout.read(&mut scratchpad[..]) {
                Ok(0) => {}
                Ok(n) => {
                    io::stdout().write_all(&scratchpad[..n])?;
                    io::stdout().flush()?;
                    stdout_buf.extend_from_slice(&scratchpad[..n]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => break Err(e),
            }
        }

        #[cfg(windows)]
        {
            use windows_pipe_utils::*;
            if can_read(&stdout)? {
                let n = read_pipe(&mut stdout, &mut scratchpad[..])?;
                if n != 0 {
                    io::stdout().write_all(&scratchpad[..n])?;
                    io::stdout().flush()?;
                    stdout_buf.extend_from_slice(&scratchpad[..n]);
                }
            }
        }

        #[cfg(unix)]
        {
            match stderr.read(&mut scratchpad[..]) {
                Ok(0) => {}
                Ok(n) => {
                    io::stderr().write_all(&scratchpad[..n])?;
                    io::stderr().flush()?;
                    stderr_buf.extend_from_slice(&scratchpad[..n]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => break Err(e),
            }
        }

        #[cfg(windows)]
        {
            use windows_pipe_utils::*;
            if can_read(&stderr)? {
                let n = read_pipe(&mut stderr, &mut scratchpad[..])?;
                if n != 0 {
                    io::stderr().write_all(&scratchpad[..n])?;
                    io::stderr().flush()?;
                    stderr_buf.extend_from_slice(&scratchpad[..n]);
                }
            }
        }

        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(exit_code)) => break Ok(Some(exit_code)),
            Err(e) => break Err(e),
        };

        std::thread::sleep(Duration::from_millis(10));
    }?
    else {
        println!("Failed execution");
        exit(1);
    };

    println!("\nChild exited with: {exit}");
    println!("Captured stdout bytes: {}", stdout_buf.len());
    println!("Captured stderr bytes: {}", stderr_buf.len());

    Ok(())
}
