//! Little-endian NIF output buffer, the counterpart of [`crate::reader::Reader`].

#[derive(Debug, Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn f32(&mut self, value: f32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn vec3(&mut self, value: [f32; 3]) {
        for v in value {
            self.f32(v);
        }
    }

    /// Block reference or string index; `None` is written as `-1`.
    pub fn index(&mut self, index: Option<usize>) {
        self.i32(index.map_or(-1, |i| i as i32));
    }

    /// `u8 length` + bytes + NUL (the length counts the NUL).
    pub fn short_string(&mut self, value: &str) {
        debug_assert!(value.len() < 255, "short string too long: {value}");
        self.u8((value.len() + 1) as u8);
        self.bytes(value.as_bytes());
        self.u8(0);
    }

    /// `u32 length` + bytes, no NUL.
    pub fn sized_string(&mut self, value: &str) {
        self.u32(value.len() as u32);
        self.bytes(value.as_bytes());
    }
}
