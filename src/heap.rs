use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::{replace, size_of};
use std::ptr::{write, NonNull};
use std::slice::from_raw_parts_mut;

use crate::allocator::{
    add_alignment_padding, AllocError, AllocHeader, AllocObject, AllocRaw, ArraySize, Mark, SizeClass,
};
use crate::bump_block::BumpBlock;
use crate::constants;
use crate::raw_ptr::RawPtr;

struct BlockList {
    head: Option<BumpBlock>,
    overflow: Option<BumpBlock>,
    free: Vec<BumpBlock>,
    recycle: Vec<BumpBlock>,
    used: Vec<BumpBlock>,
    large: Vec<()>
}

impl BlockList {
    fn new() -> BlockList {
        BlockList {
            head: None,
            overflow: None,
            free: Vec::new(),
            recycle: Vec::new(),
            used: Vec::new(),
            large: Vec::new(),
        }
    }

    pub fn block_count(&self) -> usize {
        let mut count = 0;

        if self.head.is_some() { count += 1; }
        if self.overflow.is_some() { count += 1; }
        count += self.free.len() + self.recycle.len() + self.used.len();

        count
    }

    fn overflow_alloc(&mut self, alloc_size: usize) -> Result<*const u8, AllocError> {
        assert!(alloc_size <= constants::BLOCK_CAPACITY);

        let space = match self.overflow {
            Some(ref mut overflow) => {
                match overflow.inner_alloc(alloc_size) {
                    Some(space) => space,

                    None => {
                        let free_block = if !self.free.is_empty() {
                            self.free.pop().unwrap()
                        } else {
                            BumpBlock::new()?
                        };

                        let previous = replace(overflow, free_block);

                        self.recycle.push(previous);

                        overflow.inner_alloc(alloc_size).unwrap()
                    }
                }
            }

            None => {
                let mut overflow = self.get_free_block()?;
                let space = overflow
                    .inner_alloc(alloc_size)
                    .unwrap();

                self.overflow = Some(overflow);

                space
            }
        } as *const u8;

        Ok(space)
    }

    fn get_free_block(&mut self) -> Result<BumpBlock, AllocError> {
        if !self.free.is_empty() {
            Ok(self.free.pop().unwrap())
        } else {
            BumpBlock::new()
        }
    }

    fn get_recycle_block(&mut self) -> Result<BumpBlock, AllocError> {
        if !self.recycle.is_empty() {
            Ok(self.recycle.pop().unwrap())
        } else if !self.free.is_empty() {
            Ok(self.free.pop().unwrap())
        } else {
            BumpBlock::new()
        }
    }
}

pub struct ZapHeap<H> {
    blocks: UnsafeCell<BlockList>,
    _header_type: PhantomData<*const H>,
}

impl<H> ZapHeap<H> {
    pub fn new() -> ZapHeap<H> {
        ZapHeap {
            blocks: UnsafeCell::new(BlockList::new()),
            _header_type: PhantomData,
        }
    }

    fn find_space(
        &self,
        alloc_size: usize,
        size_class: SizeClass,
    ) -> Result<*const u8, AllocError> {
        let blocks = unsafe { &mut *self.blocks.get() };

        if size_class == SizeClass::Large {
            return Err(AllocError::BadRequest);
        }

        let space = match blocks.head {
            Some(ref mut head) => {
                if size_class == SizeClass::Medium && alloc_size > head.current_hole_size() {
                    return blocks.overflow_alloc(alloc_size);
                }

                match head.inner_alloc(alloc_size) {
                    Some(space) => space,

                    None => {
                        let free_block = if !blocks.recycle.is_empty() {
                            blocks.recycle.pop().unwrap()
                        } else if !blocks.free.is_empty() {
                            blocks.free.pop().unwrap()
                        } else {
                            BumpBlock::new()?
                        };

                        let previous = replace(head, free_block);

                        blocks.used.push(previous);

                        return self.find_space(alloc_size, size_class);
                    }
                }
            }

            None => {
                let mut head = blocks.get_free_block()?;
                let space = head
                    .inner_alloc(alloc_size)
                    .unwrap();

                blocks.head = Some(head);

                space
            }
        } as *const u8;

        Ok(space)
    }
}

impl<H: AllocHeader> AllocRaw for ZapHeap<H> {
    type Header = H;

    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, AllocError>
    where
        T: AllocObject<<Self::Header as AllocHeader>::TypeId>,
    {
        let header_size = size_of::<Self::Header>();
        let header_alloc_size = add_alignment_padding(header_size);
        let object_size = size_of::<T>();
        let total_size = header_alloc_size + object_size;
        let alloc_size = add_alignment_padding(total_size);
        let size_class = SizeClass::get_for_size(alloc_size)?;
        let space = self.find_space(alloc_size, size_class)?;
        let header = Self::Header::new::<T>(object_size as ArraySize, size_class, Mark::Allocated);

        unsafe {
            let object_space = space.offset(header_alloc_size as isize);

            write(space as *mut Self::Header, header);
            write(object_space as *mut T, object);

            Ok(RawPtr::new(object_space as *const T))
        }
    }

    fn alloc_array(&self, size_bytes: ArraySize) -> Result<RawPtr<u8>, AllocError> {
        let header_size = size_of::<Self::Header>();
        let header_alloc_size = add_alignment_padding(header_size);
        let total_size = header_alloc_size + size_bytes as usize;
        let alloc_size = add_alignment_padding(total_size);
        let size_class = SizeClass::get_for_size(alloc_size)?;
        let space = self.find_space(alloc_size, size_class)?;
        let header = Self::Header::new_array(size_bytes, size_class, Mark::Allocated);

        unsafe {
            let array_space = space.offset(header_size as isize);
            write(space as *mut Self::Header, header);
            let array = from_raw_parts_mut(array_space as *mut u8, size_bytes as usize);
            for byte in array {
                *byte = 0;
            }

            Ok(RawPtr::new(array_space as *const u8))
        }
    }

    fn get_header(object: NonNull<()>) -> NonNull<Self::Header> {
        unsafe { NonNull::new_unchecked(object.cast::<Self::Header>().as_ptr().offset(-1)) }
    }

    fn get_object(header: NonNull<Self::Header>) -> NonNull<()> {
        unsafe { NonNull::new_unchecked(header.as_ptr().offset(1).cast::<()>()) }
    }
}

impl<H> Default for ZapHeap<H> {
    fn default() -> ZapHeap<H> {
        ZapHeap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::{AllocTypeId, SizeClass};

    struct SmallTestObj {
        data: u32,
    }

    struct MediumTestObj {
        data: [u8; 256],
    }

    fn alloc_size<T>() -> usize {
        let header_size = size_of::<TestHeader>();
        let header_alloc_size = add_alignment_padding(header_size);
        let object_size = size_of::<T>();
        let total_size = header_alloc_size + object_size;
        let alloc_size = add_alignment_padding(total_size);
        /*
            let alignment = size_of::<usize>(); 
            println!("ALIGNMENT: {}", alignment);       // 8
            println!("HEADER_SIZE: {}", header_size);   // 8
            println!("HEADER_ALLOC_SIZE: {}", header_alloc_size);   // 8
            println!("OBJECT_SIZE: {}", object_size);   // 4
            println!("TOTAL_SIZE:  {}",  total_size);   // 12
            println!("ALLOC_SIZE:  {}",  alloc_size);   // 16
        */
        alloc_size
    }

    impl AllocObject<TestTypeId> for MediumTestObj {
        const TYPE_ID: TestTypeId = TestTypeId::Medium;
    }

    impl AllocObject<TestTypeId> for SmallTestObj {
        const TYPE_ID: TestTypeId = TestTypeId::Small;
    }

    #[derive(PartialEq, Copy, Clone)]
    enum TestTypeId {
        Small,
        Medium,
        Large,
        Array,
    }

    impl AllocTypeId for TestTypeId {}

    struct TestHeader {
        mark: Mark,
        type_id: TestTypeId,
        size: u32,
        size_class: SizeClass
    }

    impl AllocHeader for TestHeader {
        type TypeId = TestTypeId;

        fn new<O: AllocObject<Self::TypeId>>(size: u32, size_class: SizeClass, mark: Mark) -> Self {
            TestHeader {
                type_id: O::TYPE_ID,
                mark,
                size,
                size_class
            }
        }

        fn new_array(size: u32, size_class: SizeClass, mark: Mark) -> Self {
            TestHeader {
                type_id: TestTypeId::Array,
                mark,
                size,
                size_class
            }
        }
        fn mark(&mut self) {
            self.mark = Mark::Marked;
        }

        fn is_marked(&self) -> bool {
            self.mark == Mark::Marked
        }

        fn type_id(&self) -> Self::TypeId {
            self.type_id
        }

        fn size(&self) -> u32 {
            self.size }

        fn size_class(&self) -> SizeClass {
            self.size_class
        }
    }

    #[test]
    fn test_alloc_small_obj() {
        let heap = ZapHeap::<TestHeader>::new();
        let small_obj = SmallTestObj { data: 333};
        let ptr = heap.alloc(small_obj);
        let small_obj_copy = unsafe { &*(ptr.unwrap().as_ptr()) };
        let blocks = unsafe { &mut *heap.blocks.get() };
        let alloc_size = alloc_size::<SmallTestObj>();

        assert!(small_obj_copy.data == 333);
        assert!(blocks.block_count() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.head.as_ref().unwrap().current_hole_size() == constants::BLOCK_CAPACITY - alloc_size);
    }

    #[test]
    fn test_alloc_many_small_obj() {
        let heap = ZapHeap::<TestHeader>::new();
        let blocks = unsafe { &mut *heap.blocks.get() };
        let alloc_size = alloc_size::<SmallTestObj>();

        for _ in 0..(constants::BLOCK_CAPACITY / alloc_size) {
            let small_obj = SmallTestObj { data: 333};
            let ptr = heap.alloc(small_obj);
            let small_obj_copy = unsafe { &*(ptr.unwrap().as_ptr()) };
            assert!(small_obj_copy.data == 333);
        }

        assert!(blocks.block_count() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.used.len() == 0);
        assert!(blocks.head.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY % alloc_size));

        let small_obj = SmallTestObj { data: 333};
        let ptr = heap.alloc(small_obj);
        let small_obj_copy = unsafe { &*(ptr.unwrap().as_ptr()) };

        assert!(small_obj_copy.data == 333);
        assert!(blocks.block_count() == 2);
        assert!(blocks.used.len() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.head.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY - alloc_size));
    }

    #[test]
    fn test_small_obj_header() {
        let heap = ZapHeap::<TestHeader>::new();
        let small_obj = SmallTestObj { data: 333};
        let raw_ptr = heap.alloc(small_obj).unwrap();
        let header_ptr = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let header: &TestHeader = unsafe { &*header_ptr.as_ptr() };

        assert!(header.type_id == TestTypeId::Small);
        assert!(header.size == size_of::<SmallTestObj>() as u32);
        assert!(header.size_class == SizeClass::Small);
        assert!(header.mark == Mark::Allocated);
    }

    #[test]
    fn test_get_object() {
        let heap = ZapHeap::<TestHeader>::new();
        let small_obj = SmallTestObj { data: 333};
        let raw_ptr = heap.alloc(small_obj).unwrap();
        let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let obj_ptr = ZapHeap::get_object(header_ptr);
        let obj = unsafe { &*(obj_ptr.as_ptr() as *const SmallTestObj) };

        assert!(obj.data == 333);
    }

    #[test]
    fn test_alloc_medium_object() {
        let heap = ZapHeap::<TestHeader>::new();
        let small_obj = MediumTestObj { data: [9; 256] };
        let raw_ptr = heap.alloc(small_obj).unwrap();
        let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let obj_ptr = ZapHeap::get_object(header_ptr);
        let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

        assert!(obj.data == [9; 256]);
    }

    #[test]
    fn test_over_flow_alloc() {
        let heap = ZapHeap::<TestHeader>::new();
        let blocks = unsafe { &mut *heap.blocks.get() };
        let alloc_size = alloc_size::<MediumTestObj>();

        for _ in 0..(constants::BLOCK_CAPACITY / alloc_size) {
            let medium_obj = MediumTestObj { data: [9; 256] };
            let raw_ptr = heap.alloc(medium_obj).unwrap();
            let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
            let obj_ptr = ZapHeap::get_object(header_ptr);
            let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

            assert!(obj.data == [9; 256]);
        }

        assert!(blocks.block_count() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.recycle.len() == 0);
        assert!(blocks.head.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY % alloc_size));

        let medium_obj = MediumTestObj { data: [9; 256] };
        let raw_ptr = heap.alloc(medium_obj).unwrap();
        let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let obj_ptr = ZapHeap::get_object(header_ptr);
        let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

        assert!(obj.data == [9; 256]);
        assert!(blocks.block_count() == 2);
        assert!(blocks.overflow.is_some());
        assert!(blocks.head.is_some());
        assert!(blocks.overflow.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY - alloc_size));
    }

    #[test]
    fn test_use_recycling() {
        let heap = ZapHeap::<TestHeader>::new();
        let blocks = unsafe { &mut *heap.blocks.get() };
        let alloc_size = alloc_size::<MediumTestObj>();

        for _ in 0..(constants::BLOCK_CAPACITY / alloc_size) {
            let medium_obj = MediumTestObj { data: [9; 256] };
            let raw_ptr = heap.alloc(medium_obj).unwrap();
            let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
            let obj_ptr = ZapHeap::get_object(header_ptr);
            let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

            assert!(obj.data == [9; 256]);
        }

        assert!(blocks.block_count() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.recycle.len() == 0);
        assert!(blocks.head.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY % alloc_size));

        for _ in 0..(constants::BLOCK_CAPACITY / alloc_size) {
            let medium_obj = MediumTestObj { data: [9; 256] };
            let raw_ptr = heap.alloc(medium_obj).unwrap();
            let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
            let obj_ptr = ZapHeap::get_object(header_ptr);
            let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

            assert!(obj.data == [9; 256]);
        }

        assert!(blocks.block_count() == 2);
        assert!(blocks.overflow.is_some());
        assert!(blocks.head.is_some());
        assert!(blocks.overflow.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY % alloc_size));

        let medium_obj = MediumTestObj { data: [9; 256] };
        let raw_ptr = heap.alloc(medium_obj).unwrap();
        let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let obj_ptr = ZapHeap::get_object(header_ptr);
        let obj = unsafe { &*(obj_ptr.as_ptr() as *const MediumTestObj) };

        assert!(obj.data == [9; 256]);
        assert!(blocks.block_count() == 3);
        assert!(blocks.overflow.is_some());
        assert!(blocks.recycle.len() == 1);
        assert!(blocks.head.is_some());
        assert!(blocks.overflow.as_ref().unwrap().current_hole_size() == (constants::BLOCK_CAPACITY - alloc_size));
    }

    #[test]
    fn test_array_alloc() {
        let heap = ZapHeap::<TestHeader>::new();
        let blocks = unsafe { &mut *heap.blocks.get() };
        let alloc_size = size_of::<MediumTestObj>() as u32;
        let raw_ptr = heap.alloc_array(alloc_size as u32).unwrap();
        let header_ptr: NonNull<TestHeader> = ZapHeap::get_header(raw_ptr.as_untyped()); 
        let header = unsafe { &*header_ptr.as_ptr() };

        assert!(header.type_id == TestTypeId::Array);
        assert!(header.size_class == SizeClass::Medium);
        assert!(header.mark == Mark::Allocated);
        assert!(header.size == alloc_size);
    }
}
