//! Raw V4L2 through `libc` ioctls (no crate): enumerate `/dev/video*`, negotiate MJPEG or
//! YUYV, stream with mmap'd buffers. Linux only; other platforms use the mock.

use std::ffi::CString;
use std::time::Duration;

use super::backend::*;
use super::convert::Raw;

// ---- kernel structs (64-bit layouts; sizes are asserted below)

#[repr(C)]
#[derive(Default)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Default)]
struct FmtDesc {
    index: u32,
    type_: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct Format {
    type_: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 152],
}

#[repr(C)]
#[derive(Default)]
struct RequestBuffers {
    count: u32,
    type_: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

#[repr(C)]
struct Timecode {
    type_: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
}

#[repr(C)]
struct Buffer {
    index: u32,
    type_: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: libc::timeval,
    timecode: Timecode,
    sequence: u32,
    memory: u32,
    m: u64, // offset (u32) for mmap buffers
    length: u32,
    reserved2: u32,
    request_fd: u32,
}

#[repr(C)]
#[derive(Default)]
struct FrmSizeEnum {
    index: u32,
    pixel_format: u32,
    type_: u32,
    // discrete: width, height; stepwise: min_w, max_w, step_w, min_h, max_h, step_h
    u: [u32; 6],
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Default)]
struct Control {
    id: u32,
    value: i32,
}

#[repr(C)]
#[derive(Default)]
struct QueryCtrl {
    id: u32,
    type_: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<Capability>() == 104);
    assert!(std::mem::size_of::<FmtDesc>() == 64);
    assert!(std::mem::size_of::<Format>() == 208);
    assert!(std::mem::size_of::<RequestBuffers>() == 20);
    assert!(std::mem::size_of::<Buffer>() == 88);
    assert!(std::mem::size_of::<FrmSizeEnum>() == 44);
    assert!(std::mem::size_of::<Control>() == 8);
    assert!(std::mem::size_of::<QueryCtrl>() == 68);
};

// ---- ioctl numbers (_IOC: dir << 30 | size << 16 | type << 8 | nr)

const fn ioc(dir: u32, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr) as libc::c_ulong
}
const R: u32 = 2;
const W: u32 = 1;
const VIDIOC_QUERYCAP: libc::c_ulong = ioc(R, 0, std::mem::size_of::<Capability>());
const VIDIOC_ENUM_FMT: libc::c_ulong = ioc(R | W, 2, std::mem::size_of::<FmtDesc>());
const VIDIOC_S_FMT: libc::c_ulong = ioc(R | W, 5, std::mem::size_of::<Format>());
const VIDIOC_REQBUFS: libc::c_ulong = ioc(R | W, 8, std::mem::size_of::<RequestBuffers>());
const VIDIOC_QUERYBUF: libc::c_ulong = ioc(R | W, 9, std::mem::size_of::<Buffer>());
const VIDIOC_QBUF: libc::c_ulong = ioc(R | W, 15, std::mem::size_of::<Buffer>());
const VIDIOC_DQBUF: libc::c_ulong = ioc(R | W, 17, std::mem::size_of::<Buffer>());
const VIDIOC_STREAMON: libc::c_ulong = ioc(W, 18, 4);
const VIDIOC_STREAMOFF: libc::c_ulong = ioc(W, 19, 4);
const VIDIOC_G_CTRL: libc::c_ulong = ioc(R | W, 27, std::mem::size_of::<Control>());
const VIDIOC_S_CTRL: libc::c_ulong = ioc(R | W, 28, std::mem::size_of::<Control>());
const VIDIOC_QUERYCTRL: libc::c_ulong = ioc(R | W, 36, std::mem::size_of::<QueryCtrl>());
const VIDIOC_ENUM_FRAMESIZES: libc::c_ulong = ioc(R | W, 74, std::mem::size_of::<FrmSizeEnum>());

const BUF_TYPE_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FIELD_NONE: u32 = 1;
const CID_BRIGHTNESS: u32 = 0x0098_0900;
const CID_CONTRAST: u32 = 0x0098_0901;
const CTRL_DISABLED: u32 = 0x0001;

fn ioctl<T>(fd: i32, req: libc::c_ulong, arg: &mut T) -> Result<(), i32> {
    loop {
        let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
        if r >= 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if e != libc::EINTR {
            return Err(e);
        }
    }
}

fn cstr(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned()
}

fn errno_text(e: i32) -> String {
    match e {
        libc::EACCES | libc::EPERM => "permission denied (is the user in the 'video' group?)".into(),
        libc::EBUSY => "busy (in use by another program)".into(),
        libc::ENODEV | libc::ENXIO => "gone (unplugged?)".into(),
        _ => std::io::Error::from_raw_os_error(e).to_string(),
    }
}

struct Fd(i32);
impl Fd {
    fn open(path: &str) -> Result<Fd, i32> {
        let c = CString::new(path).map_err(|_| libc::EINVAL)?;
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        } else {
            Ok(Fd(fd))
        }
    }
}
impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

fn query_caps(fd: &Fd) -> Result<Caps, i32> {
    let mut cap = Capability::default();
    ioctl(fd.0, VIDIOC_QUERYCAP, &mut cap)?;
    let mut formats = Vec::new();
    for index in 0..64 {
        let mut d = FmtDesc { index, type_: BUF_TYPE_CAPTURE, ..Default::default() };
        if ioctl(fd.0, VIDIOC_ENUM_FMT, &mut d).is_err() {
            break;
        }
        formats.push(d.pixelformat);
    }
    Ok(Caps { driver: cstr(&cap.driver), card: cstr(&cap.card), bus_info: cstr(&cap.bus_info), capabilities: cap.capabilities, device_caps: cap.device_caps, formats })
}

fn frame_sizes(fd: &Fd, fmt: u32) -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    for index in 0..64 {
        let mut s = FrmSizeEnum { index, pixel_format: fmt, ..Default::default() };
        if ioctl(fd.0, VIDIOC_ENUM_FRAMESIZES, &mut s).is_err() {
            break;
        }
        match s.type_ {
            1 => v.push((s.u[0], s.u[1])),
            _ => {
                // continuous / stepwise: offer the common sizes inside the range
                for &(w, h) in &[(160, 120), (320, 240), (640, 480), (800, 600), (1280, 720), (1280, 960)] {
                    if w >= s.u[0] && w <= s.u[1] && h >= s.u[3] && h <= s.u[4] {
                        v.push((w, h));
                    }
                }
                break;
            }
        }
    }
    v
}

pub struct V4l2;

fn video_nodes() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev")
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("video") && n[5..].chars().all(|c| c.is_ascii_digit()) && n.len() > 5).map(|n| format!("/dev/{n}")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

impl Backend for V4l2 {
    fn probe(&self) -> Probe {
        let mut p = Probe::default();
        let nodes = video_nodes();
        if nodes.is_empty() {
            p.notes.push("/dev/video*: none".into());
        }
        for path in nodes {
            let fd = match Fd::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    p.notes.push(format!("{}: {}", path.trim_start_matches("/dev/"), errno_text(e)));
                    continue;
                }
            };
            let caps = match query_caps(&fd) {
                Ok(c) => c,
                Err(e) => {
                    p.notes.push(format!("{}: {}", path.trim_start_matches("/dev/"), errno_text(e)));
                    continue;
                }
            };
            if let Err(why) = classify(&path, &caps) {
                p.notes.push(format!("{}: {why}", path.trim_start_matches("/dev/")));
                continue;
            }
            let mut sizes = Vec::new();
            for &f in &[MJPG, YUYV] {
                if caps.formats.contains(&f) {
                    sizes.extend(frame_sizes(&fd, f));
                }
            }
            let sizes = offer_sizes(&sizes);
            p.notes.push(format!("{}: {} ok", path.trim_start_matches("/dev/"), caps.card));
            p.devices.push(Device { path, name: caps.card.clone(), uvc: caps.driver == "uvcvideo", formats: caps.formats, sizes });
        }
        sort_devices(&mut p.devices);
        p
    }

    fn open(&self, dev: &Device, size: Option<(u32, u32)>, prefer_mjpeg: bool) -> Result<Box<dyn Source>, String> {
        let (w, h) = size.or_else(|| default_size(&dev.sizes)).unwrap_or((640, 480));
        let mut order = Vec::new();
        if prefer_mjpeg && dev.formats.contains(&MJPG) {
            order.push(MJPG);
        }
        if dev.formats.contains(&YUYV) {
            order.push(YUYV);
        }
        if order.is_empty() {
            return Err("no readable pixel format (MJPG/YUYV)".into());
        }
        let mut last = String::new();
        for fmt in order {
            match Fd::open(&dev.path).map_err(|e| format!("{}: {}", dev.path, errno_text(e))).and_then(|fd| Stream::start(fd, fmt, w, h)) {
                Ok(s) => return Ok(Box::new(s)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

struct Mapped {
    ptr: *mut libc::c_void,
    len: usize,
}

pub struct Stream {
    fd: Fd,
    fmt: u32,
    w: u32,
    h: u32,
    bufs: Vec<Mapped>,
    on: bool,
}

unsafe impl Send for Stream {}

impl Stream {
    fn start(fd: Fd, fmt: u32, w: u32, h: u32) -> Result<Stream, String> {
        let mut f = Format { type_: BUF_TYPE_CAPTURE, _pad: 0, pix: PixFormat { width: w, height: h, pixelformat: fmt, field: FIELD_NONE, ..Default::default() }, _rest: [0; 152] };
        ioctl(fd.0, VIDIOC_S_FMT, &mut f).map_err(|e| format!("set format: {}", errno_text(e)))?;
        if f.pix.pixelformat != fmt {
            return Err(format!("camera refused {}", fourcc_name(fmt)));
        }
        let mut s = Stream { fd, fmt, w: f.pix.width, h: f.pix.height, bufs: Vec::new(), on: false };
        let mut rb = RequestBuffers { count: 4, type_: BUF_TYPE_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
        ioctl(s.fd.0, VIDIOC_REQBUFS, &mut rb).map_err(|e| format!("buffers: {}", errno_text(e)))?;
        if rb.count < 2 {
            return Err("camera gave too few buffers".into());
        }
        for index in 0..rb.count {
            let mut b: Buffer = unsafe { std::mem::zeroed() };
            b.index = index;
            b.type_ = BUF_TYPE_CAPTURE;
            b.memory = MEMORY_MMAP;
            ioctl(s.fd.0, VIDIOC_QUERYBUF, &mut b).map_err(|e| format!("query buffer: {}", errno_text(e)))?;
            let ptr = unsafe { libc::mmap(std::ptr::null_mut(), b.length as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, s.fd.0, (b.m as u32) as libc::off_t) };
            if ptr == libc::MAP_FAILED {
                return Err("mmap failed".into());
            }
            s.bufs.push(Mapped { ptr, len: b.length as usize });
            ioctl(s.fd.0, VIDIOC_QBUF, &mut b).map_err(|e| format!("queue buffer: {}", errno_text(e)))?;
        }
        let mut t = BUF_TYPE_CAPTURE as i32;
        ioctl(s.fd.0, VIDIOC_STREAMON, &mut t).map_err(|e| format!("stream on: {}", errno_text(e)))?;
        s.on = true;
        Ok(s)
    }

    fn dequeue(&mut self) -> Result<Option<Buffer>, String> {
        let mut b: Buffer = unsafe { std::mem::zeroed() };
        b.type_ = BUF_TYPE_CAPTURE;
        b.memory = MEMORY_MMAP;
        match ioctl(self.fd.0, VIDIOC_DQBUF, &mut b) {
            Ok(()) => Ok(Some(b)),
            Err(libc::EAGAIN) => Ok(None),
            Err(e) => Err(errno_text(e)),
        }
    }

    fn requeue(&mut self, mut b: Buffer) {
        let _ = ioctl(self.fd.0, VIDIOC_QBUF, &mut b);
    }

    fn wait(&self, timeout: Duration) -> Result<bool, String> {
        let mut p = libc::pollfd { fd: self.fd.0, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut p, 1, timeout.as_millis() as i32) };
        if r < 0 {
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            return if e == libc::EINTR { Ok(false) } else { Err(errno_text(e)) };
        }
        if p.revents & (libc::POLLERR | libc::POLLHUP) != 0 && p.revents & libc::POLLIN == 0 {
            return Err("camera disconnected".into());
        }
        Ok(r > 0)
    }

    fn query(&self, id: u32) -> Option<QueryCtrl> {
        let mut q = QueryCtrl { id, ..Default::default() };
        ioctl(self.fd.0, VIDIOC_QUERYCTRL, &mut q).ok()?;
        (q.flags & CTRL_DISABLED == 0 && q.maximum > q.minimum).then_some(q)
    }
}

fn cid(c: Ctrl) -> u32 {
    match c {
        Ctrl::Brightness => CID_BRIGHTNESS,
        Ctrl::Contrast => CID_CONTRAST,
    }
}

impl Source for Stream {
    fn describe(&self) -> (String, u32, u32) {
        (fourcc_name(self.fmt), self.w, self.h)
    }

    fn next(&mut self, timeout: Duration) -> Result<Option<Raw>, String> {
        if !self.wait(timeout)? {
            return Ok(None);
        }
        let Some(mut newest) = self.dequeue()? else { return Ok(None) };
        // Drop frames: if more are waiting, hand back the older ones and keep the newest.
        while let Some(later) = self.dequeue()? {
            let old = std::mem::replace(&mut newest, later);
            self.requeue(old);
        }
        let m = &self.bufs[newest.index as usize];
        let used = (newest.bytesused as usize).min(m.len);
        let data = unsafe { std::slice::from_raw_parts(m.ptr as *const u8, used) }.to_vec();
        let (w, h, fmt) = (self.w, self.h, self.fmt);
        self.requeue(newest);
        if used == 0 {
            return Ok(None);
        }
        Ok(Some(if fmt == MJPG { Raw::Mjpeg(data) } else { Raw::Yuyv { w, h, data } }))
    }

    fn ctrl(&mut self, c: Ctrl) -> Option<CtrlRange> {
        let q = self.query(cid(c))?;
        let mut g = Control { id: cid(c), value: 0 };
        ioctl(self.fd.0, VIDIOC_G_CTRL, &mut g).ok()?;
        Some(CtrlRange { min: q.minimum, max: q.maximum, step: q.step, value: g.value })
    }

    fn set_ctrl(&mut self, c: Ctrl, v: i32) {
        let mut s = Control { id: cid(c), value: v };
        let _ = ioctl(self.fd.0, VIDIOC_S_CTRL, &mut s);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.on {
            let mut t = BUF_TYPE_CAPTURE as i32;
            let _ = ioctl(self.fd.0, VIDIOC_STREAMOFF, &mut t);
        }
        for m in &self.bufs {
            unsafe { libc::munmap(m.ptr, m.len) };
        }
        let mut rb = RequestBuffers { count: 0, type_: BUF_TYPE_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
        let _ = ioctl(self.fd.0, VIDIOC_REQBUFS, &mut rb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        // Values from <linux/videodev2.h> on 64-bit Linux.
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        assert_eq!(VIDIOC_ENUM_FMT, 0xC040_5602);
        assert_eq!(VIDIOC_S_FMT, 0xC0D0_5605);
        assert_eq!(VIDIOC_REQBUFS, 0xC014_5608);
        assert_eq!(VIDIOC_QUERYBUF, 0xC058_5609);
        assert_eq!(VIDIOC_QBUF, 0xC058_560F);
        assert_eq!(VIDIOC_DQBUF, 0xC058_5611);
        assert_eq!(VIDIOC_STREAMON, 0x4004_5612);
        assert_eq!(VIDIOC_STREAMOFF, 0x4004_5613);
        assert_eq!(VIDIOC_G_CTRL, 0xC008_561B);
        assert_eq!(VIDIOC_S_CTRL, 0xC008_561C);
        assert_eq!(VIDIOC_QUERYCTRL, 0xC044_5624);
        assert_eq!(VIDIOC_ENUM_FRAMESIZES, 0xC02C_564A);
    }
}
