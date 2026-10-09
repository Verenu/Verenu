//! Bounded, cancellable exchange with the private CoreML helper.
use std::{
    io::{Read, Write},
    os::fd::AsRawFd,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) fn exchange(
    input: &mut (impl Write + AsRawFd),
    output: &mut (impl Read + AsRawFd),
    payload: &[u8],
    cancel: &AtomicBool,
    started: Instant,
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    for fd in [input.as_raw_fd(), output.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let check = || -> anyhow::Result<()> {
        anyhow::ensure!(
            !cancel.load(Ordering::Acquire) && started.elapsed() < timeout,
            "FluidAudio operation cancelled or timed out"
        );
        Ok(())
    };
    let wait = |fd, events| -> anyhow::Result<()> {
        check()?;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let remaining = timeout.saturating_sub(started.elapsed());
        let milliseconds = remaining.as_millis().min(50) as i32;
        let ready = unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
        check()
    };
    let mut written = 0;
    while written < payload.len() {
        check()?;
        match input.write(&payload[written..]) {
            Ok(0) => anyhow::bail!("FluidAudio helper stopped reading"),
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait(input.as_raw_fd(), libc::POLLOUT)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut response = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        check()?;
        match output.read(&mut buffer) {
            Ok(0) => anyhow::bail!("FluidAudio helper stopped"),
            Ok(count) => {
                response.extend_from_slice(&buffer[..count]);
                if response.contains(&b'\n') {
                    return Ok(response);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait(output.as_raw_fd(), libc::POLLIN)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn stalled_helper_write_obeys_deadline() {
        let (mut input, _helper) = UnixStream::pair().unwrap();
        let mut output = input.try_clone().unwrap();
        let error = exchange(
            &mut input,
            &mut output,
            &vec![0; 2_000_000],
            &AtomicBool::new(false),
            Instant::now(),
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn stalled_helper_write_observes_cancellation() {
        let (mut input, mut helper) = UnixStream::pair().unwrap();
        let mut output = input.try_clone().unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let worker = std::thread::spawn(move || {
            helper.read_exact(&mut [0]).unwrap();
            flag.store(true, Ordering::Release);
            helper
        });
        let error = exchange(
            &mut input,
            &mut output,
            &vec![0; 2_000_000],
            &cancel,
            Instant::now(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        let _helper = worker.join().unwrap();
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn partial_helper_response_obeys_deadline() {
        let (mut input, mut helper) = UnixStream::pair().unwrap();
        let mut output = input.try_clone().unwrap();
        helper.write_all(b"{\"text\":").unwrap();
        let error = exchange(
            &mut input,
            &mut output,
            b"request\n",
            &AtomicBool::new(false),
            Instant::now(),
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn complete_helper_response_is_retained() {
        let (mut input, mut helper) = UnixStream::pair().unwrap();
        let mut output = input.try_clone().unwrap();
        helper.write_all(b"{\"text\":\"synthetic\"}\n").unwrap();
        let response = exchange(
            &mut input,
            &mut output,
            b"request\n",
            &AtomicBool::new(false),
            Instant::now(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(response, b"{\"text\":\"synthetic\"}\n");
        let mut request = [0; 8];
        helper.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"request\n");
    }
}
