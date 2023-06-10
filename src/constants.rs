use std::mem::size_of;

pub const BLOCK_SIZE: usize = 1024 * 16;
pub const LINE_SIZE: usize = 128;
pub const META_SIZE: usize = BLOCK_SIZE / LINE_SIZE;
pub const LINE_COUNT: usize = META_SIZE - 1;
pub const BLOCK_CAPACITY: usize = BLOCK_SIZE - META_SIZE;
pub const META_OFFSET: usize = BLOCK_CAPACITY;

pub const ALLOC_ALIGN_MASK: usize = !(size_of::<usize>() - 1);

pub const FREE: u8 = 0;
pub const MARKED: u8 = 1;
 
pub const MAX_ALLOC_SIZE: usize = std::u32::MAX as usize;
pub const SMALL_OBJECT_MIN: usize = 1;
pub const SMALL_OBJECT_MAX: usize = LINE_SIZE;
pub const MEDIUM_OBJECT_MIN: usize = SMALL_OBJECT_MAX + 1;
pub const MEDIUM_OBJECT_MAX: usize = 1024 * 8;
pub const LARGE_OBJECT_MIN: usize = MEDIUM_OBJECT_MAX + 1;
pub const LARGE_OBJECT_MAX: usize = MAX_ALLOC_SIZE;
