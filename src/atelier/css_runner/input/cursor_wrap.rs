//! Cursor recentering for motion-based drags, confined to an active native drag.
//! Synthetic warp motion is excluded by resetting the sampling origin.

pub(in super::super) trait PointerApi {
    fn position(&self) -> Option<(i32, i32)>;
    fn vertical_bounds(&self, x: i32, y: i32) -> Option<(i32, i32)>;
    fn warp(&self, x: i32, y: i32) -> bool;
}
impl PointerApi for platform::Pointer {
    fn position(&self) -> Option<(i32, i32)> {
        self.position()
    }
    fn vertical_bounds(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        self.vertical_bounds(x, y)
    }
    fn warp(&self, x: i32, y: i32) -> bool {
        self.warp(x, y)
    }
}
pub(in super::super) struct CursorWrap<P: PointerApi = platform::Pointer> {
    pointer: P,
    origin: (i32, i32),
    last_y: i32,
}
impl CursorWrap {
    pub fn new() -> Option<Self> {
        Self::with_pointer(platform::Pointer::new()?)
    }
}
impl<P: PointerApi> CursorWrap<P> {
    fn with_pointer(pointer: P) -> Option<Self> {
        let origin = pointer.position()?;
        Some(Self {
            pointer,
            origin,
            last_y: origin.1,
        })
    }
    pub fn sample(&mut self) -> Option<f64> {
        let (x, y) = self.pointer.position()?;
        let dy = self.last_y - y;
        self.last_y = y;
        let (top, bottom) = self.pointer.vertical_bounds(x, y)?;
        if y <= top + 8 || y >= bottom - 9 {
            let middle = top + (bottom - top) / 2;
            if self.pointer.warp(x, middle) {
                self.last_y = middle;
            }
        }
        Some(dy as f64)
    }
}
impl<P: PointerApi> Drop for CursorWrap<P> {
    fn drop(&mut self) {
        self.pointer.warp(self.origin.0, self.origin.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    struct Pointer(Rc<Cell<(i32, i32)>>);
    impl PointerApi for Pointer {
        fn position(&self) -> Option<(i32, i32)> {
            Some(self.0.get())
        }
        fn vertical_bounds(&self, _: i32, _: i32) -> Option<(i32, i32)> {
            Some((0, 1000))
        }
        fn warp(&self, x: i32, y: i32) -> bool {
            self.0.set((x, y));
            true
        }
    }
    #[test]
    fn wrap_ignores_synthetic_motion_and_restores_pointer() {
        let position = Rc::new(Cell::new((100, 100)));
        let mut cursor = CursorWrap::with_pointer(Pointer(position.clone())).unwrap();
        position.set((100, 0));
        assert_eq!(cursor.sample(), Some(100.0));
        assert_eq!(position.get(), (100, 500));
        assert_eq!(
            cursor.sample(),
            Some(0.0),
            "recenter must not change the number"
        );
        position.set((100, 480));
        assert_eq!(cursor.sample(), Some(20.0));
        position.set((100, 999));
        assert_eq!(cursor.sample(), Some(-519.0));
        assert_eq!(position.get(), (100, 500));
        assert_eq!(cursor.sample(), Some(0.0));
        drop(cursor);
        assert_eq!(position.get(), (100, 100));
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use x11rb::{connection::Connection, protocol::xproto::ConnectionExt};
    pub struct Pointer {
        connection: x11rb::rust_connection::RustConnection,
        root: u32,
    }
    impl Pointer {
        pub fn new() -> Option<Self> {
            // XWayland cannot promise unrestricted pointer warping. Use ordinary
            // in-window deltas there rather than silently losing physical motion.
            // The XWayland fallback removes WAYLAND_DISPLAY, so ask display_backend.
            if clothcraft::display_backend::wayland_session() {
                return None;
            }
            let (connection, screen) = x11rb::connect(None).ok()?;
            let root = connection.setup().roots[screen].root;
            Some(Self { connection, root })
        }
        pub fn position(&self) -> Option<(i32, i32)> {
            let p = self
                .connection
                .query_pointer(self.root)
                .ok()?
                .reply()
                .ok()?;
            Some((p.root_x as i32, p.root_y as i32))
        }
        pub fn vertical_bounds(&self, _: i32, _: i32) -> Option<(i32, i32)> {
            let r = self.connection.get_geometry(self.root).ok()?.reply().ok()?;
            Some((0, r.height as i32))
        }
        pub fn warp(&self, x: i32, y: i32) -> bool {
            self.connection
                .warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, x as i16, y as i16)
                .ok()
                .and_then(|c| c.check().ok())
                .is_some()
                && self.connection.flush().is_ok()
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn MonitorFromPoint(point: Point, flags: u32) -> *mut std::ffi::c_void;
        fn GetMonitorInfoW(monitor: *mut std::ffi::c_void, info: *mut MonitorInfo) -> i32;
    }
    pub struct Pointer;
    impl Pointer {
        pub fn new() -> Option<Self> {
            Some(Self)
        }
        pub fn position(&self) -> Option<(i32, i32)> {
            let mut point = Point { x: 0, y: 0 };
            // SAFETY: point is writable and has the documented POINT layout.
            (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
        }
        pub fn vertical_bounds(&self, x: i32, y: i32) -> Option<(i32, i32)> {
            let empty = || Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let mut info = MonitorInfo {
                size: std::mem::size_of::<MonitorInfo>() as u32,
                monitor: empty(),
                work: empty(),
                flags: 0,
            };
            // SAFETY: monitor handle comes from user32; info includes its size.
            unsafe {
                let monitor = MonitorFromPoint(Point { x, y }, 2);
                if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
                    return None;
                }
            }
            Some((info.monitor.top, info.monitor.bottom))
        }
        pub fn warp(&self, x: i32, y: i32) -> bool {
            // SAFETY: coordinates are finite global desktop coordinates.
            unsafe { SetCursorPos(x, y) != 0 }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *const std::ffi::c_void;
        fn CGEventGetLocation(event: *const std::ffi::c_void) -> Point;
        fn CGWarpMouseCursorPosition(point: Point) -> i32;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> Rect;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const std::ffi::c_void);
    }
    pub struct Pointer;
    impl Pointer {
        pub fn new() -> Option<Self> {
            Some(Self)
        }
        pub fn position(&self) -> Option<(i32, i32)> {
            // SAFETY: Quartz creates an owned event; read before releasing it.
            unsafe {
                let event = CGEventCreate(std::ptr::null());
                if event.is_null() {
                    return None;
                }
                let point = CGEventGetLocation(event);
                CFRelease(event);
                Some((point.x.round() as i32, point.y.round() as i32))
            }
        }
        pub fn vertical_bounds(&self, x: i32, y: i32) -> Option<(i32, i32)> {
            let mut displays = [0u32; 32];
            let mut count = 0;
            // SAFETY: array/count are writable and capacity matches the API limit.
            if unsafe { CGGetActiveDisplayList(32, displays.as_mut_ptr(), &mut count) } != 0 {
                return None;
            }
            for display in &displays[..(count as usize).min(32)] {
                let r = unsafe { CGDisplayBounds(*display) };
                if (x as f64) >= r.origin.x
                    && (x as f64) < r.origin.x + r.size.width
                    && (y as f64) >= r.origin.y
                    && (y as f64) < r.origin.y + r.size.height
                {
                    return Some((r.origin.y as i32, (r.origin.y + r.size.height) as i32));
                }
            }
            None
        }
        pub fn warp(&self, x: i32, y: i32) -> bool {
            // SAFETY: Quartz accepts a finite point in global display coordinates.
            unsafe {
                CGWarpMouseCursorPosition(Point {
                    x: x as f64,
                    y: y as f64,
                }) == 0
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod platform {
    pub struct Pointer;
    impl Pointer {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn position(&self) -> Option<(i32, i32)> {
            None
        }
        pub fn vertical_bounds(&self, _: i32, _: i32) -> Option<(i32, i32)> {
            None
        }
        pub fn warp(&self, _: i32, _: i32) -> bool {
            false
        }
    }
}
