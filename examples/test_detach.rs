use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;

fn main() {
    let pty = nix::pty::openpty(None, None).unwrap();
    let master_fd = pty.master.as_raw_fd();
    let slave_fd = pty.slave.as_raw_fd();

    // propagate size
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    unsafe { libc::ioctl(std::io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ as libc::c_ulong, &ws) };

    let mut cmd = std::process::Command::new("bash");
    cmd.arg("-c")
        .arg("echo 'Type stuff. Press Ctrl+B to detach.'; read line; echo \"You typed: $line\"");

    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0);
            libc::dup2(slave_fd, 0);
            libc::dup2(slave_fd, 1);
            libc::dup2(slave_fd, 2);
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            Ok(())
        });
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().unwrap();
    drop(pty.slave);

    // Open /dev/tty
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .unwrap();
    let tty_fd = tty.as_raw_fd();

    let orig = nix::sys::termios::tcgetattr(&tty).unwrap();
    let mut raw = orig.clone();
    nix::sys::termios::cfmakeraw(&mut raw);
    nix::sys::termios::tcsetattr(&tty, nix::sys::termios::SetArg::TCSANOW, &raw).unwrap();

    eprintln!("\r\n[test] PTY proxy running. Type keys. Ctrl+B=detach, Ctrl+C=quit.\r");

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let detached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let d1 = detached.clone();
    let dn1 = done.clone();
    let t1 = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !dn1.load(std::sync::atomic::Ordering::SeqCst) {
            let mut pfd = libc::pollfd {
                fd: tty_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut pfd, 1, 100) } <= 0 {
                continue;
            }
            let n = unsafe { libc::read(tty_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            let data = &buf[..n as usize];
            eprint!("\r[proxy] {} bytes: ", n);
            for b in data {
                eprint!("0x{:02x} ", b);
            }
            eprint!("\r\n");
            if data.contains(&0x02) {
                eprintln!("\r[proxy] Ctrl+B detected! Detaching.\r");
                d1.store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
            unsafe { libc::write(master_fd, data.as_ptr() as *const _, data.len()) };
        }
    });

    let dn2 = done.clone();
    let t2 = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !dn2.load(std::sync::atomic::Ordering::SeqCst) {
            let mut pfd = libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut pfd, 1, 100) } <= 0 {
                continue;
            }
            let n = unsafe { libc::read(master_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(&buf[..n as usize]);
            let _ = stdout.flush();
        }
    });

    loop {
        if detached.load(std::sync::atomic::Ordering::SeqCst) {
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                done.store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => {
                done.store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
        }
    }

    let _ = t1.join();
    let _ = t2.join();
    nix::sys::termios::tcsetattr(&tty, nix::sys::termios::SetArg::TCSANOW, &orig).unwrap();

    if detached.load(std::sync::atomic::Ordering::SeqCst) {
        eprintln!("\r\n[DETACHED successfully]\r");
    } else {
        eprintln!("\r\n[child exited]\r");
    }
}
