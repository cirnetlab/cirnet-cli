use super::{IoStop, PtyReader, PtyWriter};
use portable_pty::MasterPty;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, BorrowedFd},
        unix::net::UnixStream,
    },
};

pub(super) fn pty_streams(
    master: &dyn MasterPty,
) -> anyhow::Result<(PtyReader, PtyWriter, IoStop)> {
    let fd = master
        .as_raw_fd()
        .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))?;
    // We borrow the fd only while the caller still owns the PTY master.
    let owned = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()?;
    let terminal = File::from(owned);
    // A duplicate shares the open file description with the PTY master.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let (signal, stopped) = UnixStream::pair()?;
    let reader = Pipe {
        terminal: terminal.try_clone()?,
        stopped: stopped.try_clone()?,
    };
    let writer = Pipe { terminal, stopped };
    Ok((
        Box::new(reader),
        Box::new(writer),
        IoStop { _signal: signal },
    ))
}

struct Pipe {
    terminal: File,
    stopped: UnixStream,
}

impl Pipe {
    fn ready(&self, events: libc::c_short) -> io::Result<()> {
        let mut descriptors = [
            libc::pollfd {
                fd: self.stopped.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.terminal.as_raw_fd(),
                events,
                revents: 0,
            },
        ];
        // Both poll descriptors borrow handles owned by this Pipe.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if descriptors[0].revents != 0 {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        Ok(())
    }
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            self.ready(libc::POLLIN)?;
            match self.terminal.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                other => return other,
            }
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        loop {
            self.ready(libc::POLLOUT)?;
            match self.terminal.write(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                other => return other,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.terminal.flush()
    }
}
