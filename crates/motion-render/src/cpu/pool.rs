//! Bounded session-owned pixel storage; idle capacity is separate from active payload.

use motion_core::sync::MutexExt;

#[derive(Debug)]
pub(crate) struct BufferPool<Value> {
    shared: std::sync::Arc<std::sync::Mutex<Storage<Value>>>,
}

#[derive(Debug)]
struct Storage<Value> {
    buffers: Vec<Vec<Value>>,
    bytes: u64,
    maximum: u64,
}

pub(crate) type SurfacePool = BufferPool<[f32; 4]>;

impl<Value> Default for BufferPool<Value> {
    fn default() -> Self {
        Self {
            shared: std::sync::Arc::new(std::sync::Mutex::new(Storage {
                buffers: Vec::new(),
                bytes: 0,
                maximum: 0,
            })),
        }
    }
}

impl<Value> Clone for BufferPool<Value> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<Value> BufferPool<Value> {
    pub(crate) fn trim(&self, maximum: u64) {
        self.shared.lock_unpoisoned().trim(maximum);
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.shared.lock_unpoisoned().bytes
    }

    pub(crate) fn take(&self, count: usize) -> Option<Vec<Value>> {
        self.shared.lock_unpoisoned().take(count)
    }

    pub(crate) fn put(&self, buffer: Vec<Value>) {
        self.shared.lock_unpoisoned().put(buffer);
    }
}

impl<Value> Storage<Value> {
    fn trim(&mut self, maximum: u64) {
        self.maximum = maximum.min(64 * 1024 * 1024);
        while self.bytes > self.maximum {
            let buffer = self.buffers.pop().expect("retained pixel buffer");
            self.bytes -= buffer.capacity() as u64 * std::mem::size_of::<Value>() as u64;
        }
    }

    fn take(&mut self, count: usize) -> Option<Vec<Value>> {
        let index = self
            .buffers
            .iter()
            .position(|buffer| buffer.len() == count)?;
        let buffer = self.buffers.swap_remove(index);
        self.bytes -= buffer.capacity() as u64 * std::mem::size_of::<Value>() as u64;
        Some(buffer)
    }

    fn put(&mut self, buffer: Vec<Value>) {
        let bytes = buffer.capacity() as u64 * std::mem::size_of::<Value>() as u64;
        if bytes == 0 || bytes > self.maximum {
            return;
        }
        while self.bytes + bytes > self.maximum || self.buffers.len() == 4 {
            let previous = self.buffers.pop().expect("retained pixel buffer");
            self.bytes -= previous.capacity() as u64 * std::mem::size_of::<Value>() as u64;
        }
        if self.buffers.try_reserve(1).is_ok() {
            self.bytes += bytes;
            self.buffers.push(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_reuses_exact_pixel_storage_and_bounds_idle_capacity() {
        let pool = SurfacePool::default();
        pool.trim(4096);
        let buffer = vec![[0.25; 4]; 32];
        let pointer = buffer.as_ptr();
        pool.put(buffer);
        assert_eq!(pool.bytes(), 512);
        assert!(pool.take(33).is_none());
        let buffer = pool.take(32).unwrap();
        assert_eq!(buffer.as_ptr(), pointer);
        assert_eq!(pool.bytes(), 0);
        pool.put(buffer);
        pool.trim(1);
        assert_eq!(pool.bytes(), 0);
    }
}
