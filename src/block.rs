use std::alloc::{Layout, alloc, dealloc};
use std::ptr::NonNull;

pub type BlockPtr = NonNull<u8>;
pub type BlockSize = usize;

#[derive(Debug, PartialEq)]
pub enum BlockError {
    BadRequest,
    OOM,
}

pub struct Block {
    ptr: BlockPtr,
    size: BlockSize,
}

impl Block {
    pub fn new(size: BlockSize) -> Result<Block, BlockError> {
        let layout = Layout::from_size_align(size, size);

        if layout.is_err() {
            return Err(BlockError::BadRequest);
        }

        let unchecked_ptr = unsafe { alloc(layout.unwrap()) };

        if unchecked_ptr.is_null() {
            return Err(BlockError::OOM);
        }

        let ptr = unsafe { NonNull::new_unchecked(unchecked_ptr) };

        Ok(Block { ptr, size })
    }

    pub fn into_mut_ptr(self) -> BlockPtr {
        self.ptr
    }

    pub fn size(&self) -> BlockSize {
        self.size
    }

    pub unsafe fn from_raw_parts(ptr: BlockPtr, size: BlockSize) -> Block {
        Block { ptr, size }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        unsafe {
            let layout = Layout::from_size_align_unchecked(self.size, self.size);

            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_block() {
        let result = Block::new(1024).unwrap();

        assert!(result.size == 1024);
    }

    #[test]
    fn bad_request() {
        let result = Block::new(3);

        assert!(result.err().unwrap() == BlockError::BadRequest);
    }

    #[test]
    fn block_size_must_be_power_of_2() {
        let size: usize = 2;

        for i in 1..25 {
            let result = Block::new(size.pow(i)).unwrap();

            assert!(result.size == size.pow(i));
        }
    }
}
