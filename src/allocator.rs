use std::ptr::NonNull;
use std::mem::size_of;

use crate::constants;
use crate::raw_ptr::RawPtr;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AllocError {
    BadRequest,
    OOM,
}

pub trait AllocTypeId: Copy + Clone {}

pub trait AllocObject<T: AllocTypeId> {
    const TYPE_ID: T;
}

pub trait AllocHeader: Sized {
    type TypeId: AllocTypeId;

    fn new<O: AllocObject<Self::TypeId>>(size: u32, size_class: SizeClass, mark: Mark) -> Self;
    fn new_array(size: ArraySize, size_class: SizeClass, mark: Mark) -> Self;
    fn mark(&mut self);
    fn is_marked(&self) -> bool;
    fn size_class(&self) -> SizeClass;
    fn size(&self) -> u32;
    fn type_id(&self) -> Self::TypeId;
}

pub trait AllocRaw {
    type Header: AllocHeader;

    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, AllocError>
    where
        T: AllocObject<<Self::Header as AllocHeader>::TypeId>;
    fn alloc_array(&self, size_bytes: ArraySize) -> Result<RawPtr<u8>, AllocError>;
    fn get_header(object: NonNull<()>) -> NonNull<Self::Header>;
    fn get_object(header: NonNull<Self::Header>) -> NonNull<()>;
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SizeClass {
    Small,
    Medium,
    Large,
}

impl SizeClass {
    pub fn get_for_size(object_size: usize) -> Result<SizeClass, AllocError> {
        match object_size {
            constants::SMALL_OBJECT_MIN..=constants::SMALL_OBJECT_MAX => Ok(SizeClass::Small),
            constants::MEDIUM_OBJECT_MIN..=constants::MEDIUM_OBJECT_MAX => Ok(SizeClass::Medium),
            constants::LARGE_OBJECT_MIN..=constants::LARGE_OBJECT_MAX => Ok(SizeClass::Large),
            _ => Err(AllocError::BadRequest),
        }
    }
}

pub type ArraySize = u32;

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Mark {
    Allocated,
    Unmarked,
    Marked,
}

pub fn add_alignment_padding(object_size: usize) -> usize {
    let align = size_of::<usize>();

    if (object_size % align) == 0 { return object_size; }

    object_size + (align - (object_size % align))
}
