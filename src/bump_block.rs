use crate::block::{BlockError, Block};
use crate::allocator::AllocError;
use crate::constants;

use std::ptr::write;

impl From<BlockError> for AllocError {
    fn from(error: BlockError) -> AllocError {
        match error {
            BlockError::BadRequest => AllocError::BadRequest,
            BlockError::OOM => AllocError::OOM,
        }
    }
}

pub struct BumpBlock {
    block: Block,
    cursor: *const u8,
    limit: *const u8
}

impl BumpBlock {
    pub fn new() -> Result<BumpBlock, AllocError> {
        let block = Block::new(constants::BLOCK_SIZE)?;
        let limit = block.as_ptr();
        let cursor = unsafe { limit.add(constants::BLOCK_CAPACITY) };
        let mut bump_block = BumpBlock { block, cursor, limit};

        bump_block.reset();

        Ok(bump_block)
    }

    pub fn inner_alloc(&mut self, alloc_size: usize) -> Option<*const u8> {
        let ptr = self.cursor as usize;
        let limit = self.limit as usize;
        let next_ptr = ptr.checked_sub(alloc_size)? & constants::ALLOC_ALIGN_MASK;

        if next_ptr < limit {
            let block_relative_limit =
                unsafe { self.limit.sub(self.block.as_ptr() as usize) } as usize;

            if block_relative_limit > 0 {
                if let Some((cursor, limit)) = self
                    .find_next_available_hole(block_relative_limit, alloc_size)
                {
                    self.cursor = unsafe { self.block.as_ptr().add(cursor) };
                    self.limit = unsafe { self.block.as_ptr().add(limit) };
                    return self.inner_alloc(alloc_size);
                }
            }

            None
        } else {
            self.cursor = next_ptr as *const u8;
            Some(self.cursor)
        }
    }

    fn find_next_available_hole(
        &self,
        starting_at: usize,
        alloc_size: usize,
    ) -> Option<(usize, usize)> {
        let mut count = 0;
        let starting_line = starting_at / constants::LINE_SIZE;
        let lines_required = (alloc_size + constants::LINE_SIZE - 1) / constants::LINE_SIZE;
        let mut end = starting_line;

        for index in (0..starting_line).rev() {
            let marked = unsafe { *self.block.as_ptr().add(constants::META_OFFSET + index) };

            if marked == 0 {
                count += 1;

                if index == 0 && count >= lines_required {
                    let limit = 0;
                    let cursor = end * constants::LINE_SIZE;
                    return Some((cursor, limit));
                }
            } else {
                if count > lines_required {
                    let limit = (index + 2) * constants::LINE_SIZE;
                    let cursor = end * constants::LINE_SIZE;
                    return Some((cursor, limit));
                }

                count = 0;
                end = index;
            }
        }

        None
    }

    pub fn mark_line(&mut self, line_num: usize) {
        if constants::LINE_COUNT <= line_num {
            panic!("ALLOC ERROR: tried marking non existent line");
        }

        let line_marker = unsafe { self.block.as_ptr().add(constants::META_OFFSET + line_num) as *mut u8 };

        unsafe { *line_marker = constants::MARKED; };
    }

    pub fn mark_block(&mut self) {
        let block_marker = unsafe { self.block.as_ptr().add(constants::BLOCK_SIZE - 1) as *mut u8 };

        unsafe { *block_marker = constants::MARKED; };

    }

    pub fn reset(&mut self) {
        self.limit = self.block.as_ptr();
        self.cursor = unsafe { self.limit.add(constants::BLOCK_CAPACITY) };

        unsafe {
            for i in 0..128 {
                 *(self.block.as_ptr().add(constants::META_OFFSET + i) as *mut u8)
                     = constants::FREE;
            }
        }
    }

    unsafe fn write<T>(&mut self, object: T, offset: usize) -> *const T {
        let p = self.block.as_ptr().add(offset) as *mut T;
        write(p, object);
        p
    }

    pub fn current_hole_size(&self) -> usize {
        self.cursor as usize - self.limit as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn test_begins_with_full_capacity() {
        let b = BumpBlock::new().unwrap();

        assert!(b.current_hole_size() == constants::BLOCK_CAPACITY);
    }

    #[test]
    fn test_writes_obj() {
        let mut b = BumpBlock::new().unwrap();
        let important_number = 69;
        let ptr = unsafe { b.write(important_number, 420) }; 
        let val = unsafe { *ptr };

        assert!(val == important_number);
    }

    #[test]
    fn test_find_next_hole() {
        let mut block = BumpBlock::new().unwrap();

        block.mark_line(0);
        block.mark_line(1);
        block.mark_line(2);
        block.mark_line(4);
        block.mark_line(10);

        let expect = Some((10 * constants::LINE_SIZE, 6 * constants::LINE_SIZE));

        let got = block.find_next_available_hole(10 * constants::LINE_SIZE, constants::LINE_SIZE);

        println!("test_find_next_hole got {:?} expected {:?}", got, expect);

        assert!(got == expect);
    }

    #[test]
    fn test_find_next_hole_at_line_zero() {
        let mut block = BumpBlock::new().unwrap();

        block.mark_line(3);
        block.mark_line(4);
        block.mark_line(5);

        let expect = Some((3 * constants::LINE_SIZE, 0));

        let got = block.find_next_available_hole(3 * constants::LINE_SIZE, constants::LINE_SIZE);

        println!(
            "test_find_next_hole_at_line_zero got {:?} expected {:?}",
            got, expect
        );

        assert!(got == expect);
    }

    #[test]
    fn test_find_next_hole_at_block_end() {
        let mut block = BumpBlock::new().unwrap();

        let halfway = constants::LINE_COUNT / 2;

        for i in halfway..constants::LINE_COUNT {
            block.mark_line(i);
        }

        let expect = Some((halfway * constants::LINE_SIZE, 0));

        let got = block.find_next_available_hole(constants::BLOCK_CAPACITY, constants::LINE_SIZE);

        println!(
            "test_find_next_hole_at_block_end got {:?} expected {:?}",
            got, expect
        );

        assert!(got == expect);
    }


    #[test]
    fn test_find_hole_all_conservatively_marked() {
        let mut block = BumpBlock::new().unwrap();

        for i in 0..constants::LINE_COUNT {
            if i % 2 == 0 {
                block.mark_line(i);
            }
        }

        let got = block.find_next_available_hole(constants::BLOCK_CAPACITY, constants::LINE_SIZE);

        println!(
            "test_find_hole_all_conservatively_marked got {:?} expected None",
            got
        );

        assert!(got == None);
    }


    #[test]
    fn test_find_entire_block() {
        let block = BumpBlock::new().unwrap();

        let expect = Some((constants::BLOCK_CAPACITY, 0));
        let got = block.find_next_available_hole(constants::BLOCK_CAPACITY, constants::LINE_SIZE);

        println!("test_find_entire_block got {:?} expected {:?}", got, expect);

        assert!(got == expect);
    }

    #[test]
    fn test_mark_line_overflow_panics() {
        let mut block = BumpBlock::new().unwrap();

        block.mark_line(126); // line 126 is the last line

        let result = std::panic::catch_unwind(move || block.mark_line(127));

        assert!(result.is_err());
    }

    #[test]
    fn test_alloc_empty_block() {
        let mut block = BumpBlock::new().unwrap();
        let alloc_size = 8;
        let ptr = block.inner_alloc(alloc_size).unwrap();

        assert!(block.current_hole_size() == (constants::BLOCK_CAPACITY - 8));
        assert!(ptr == unsafe { block.block.as_ptr().add(constants::BLOCK_CAPACITY - 8) });
    }

    #[test]
    fn test_block_write() {
        let mut block = BumpBlock::new().unwrap();
        let my_bytes: [u8; 4] = [1, 2, 3, 4];
        let ptr = block.inner_alloc(4).unwrap();
        let ptr_offset = (ptr as usize) - (block.block.as_ptr() as usize);
        let my_written_bytes = unsafe { *block.write::<[u8; 4]>(my_bytes, ptr_offset) };

        assert!(my_written_bytes == my_bytes);
    }

    #[test]
    fn test_block_alloc_aligns_to_usize() {
        let mut block = BumpBlock::new().unwrap();
        let alloc_size = 1;
        let mut ptr = block.inner_alloc(alloc_size).unwrap();

        assert!(block.current_hole_size() == (constants::BLOCK_CAPACITY - size_of::<usize>()));
        assert!(ptr == unsafe { block.block.as_ptr().add(constants::BLOCK_CAPACITY - size_of::<usize>()) });

        ptr = block.inner_alloc(alloc_size).unwrap();

        assert!(block.current_hole_size() == (constants::BLOCK_CAPACITY - (size_of::<usize>() * 2)));
        assert!(ptr == unsafe { block.block.as_ptr().add(constants::BLOCK_CAPACITY - (size_of::<usize>() * 2)) });
    }

    #[test]
    fn test_alloc_on_full_block() {
        let mut block = BumpBlock::new().unwrap();
        let alloc_size = 128;

        for i in 1..=constants::LINE_COUNT {
            let ptr = block.inner_alloc(alloc_size).unwrap();
            let cursor = i * 128;

            assert!(block.current_hole_size() == (constants::BLOCK_CAPACITY - cursor));
            assert!(ptr == unsafe { block.block.as_ptr().add(constants::BLOCK_CAPACITY - cursor) });
        }

        let ptr = block.inner_alloc(1);
        assert!(ptr.is_none());
    }

    #[test]
    fn test_reset() {
        let mut block = BumpBlock::new().unwrap();

        block.cursor = block.limit;

        for i in 0..constants::LINE_COUNT {
            block.mark_line(i);
        }

        let ptr = block.inner_alloc(1);
        println!("{:?}", ptr);
        assert!(ptr.is_none());
block.reset();

        let ptr = block.inner_alloc(1).unwrap();
        assert!(block.current_hole_size() == (constants::BLOCK_CAPACITY - 8));
        assert!(ptr == unsafe { block.block.as_ptr().add(constants::BLOCK_CAPACITY - 8) });
    }
}
