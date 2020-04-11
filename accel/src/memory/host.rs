//! Device and Host memory handlers

use super::*;
use crate::{device::*, ffi_call, ffi_new};
use cuda::*;
use std::ops::{Deref, DerefMut};

/// Memory allocated as page-locked
pub struct PageLockedMemory<'ctx, T> {
    ptr: *mut T,
    size: usize,
    context: &'ctx Context,
}

impl<'ctx, T> Drop for PageLockedMemory<'ctx, T> {
    fn drop(&mut self) {
        if let Err(e) = ffi_call!(cuMemFreeHost, self.ptr as *mut _) {
            log::error!("Cannot free page-locked memory: {:?}", e);
        }
    }
}

impl<'ctx, T> Deref for PageLockedMemory<'ctx, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.ptr as _, self.size) }
    }
}

impl<'ctx, T> DerefMut for PageLockedMemory<'ctx, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }
}

impl<'ctx, T> Contexted for PageLockedMemory<'ctx, T> {
    fn get_context(&self) -> &Context {
        &self.context
    }
}

impl<'ctx, T: Copy> Memory for PageLockedMemory<'ctx, T> {
    type Elem = T;
    fn head_addr(&self) -> *const T {
        self.ptr as _
    }
    fn byte_size(&self) -> usize {
        self.size * std::mem::size_of::<T>()
    }
    fn try_as_slice(&self) -> Option<&[T]> {
        Some(self.as_slice())
    }
    fn try_get_context(&self) -> Option<&Context> {
        Some(self.get_context())
    }
    fn memory_type(&self) -> MemoryType {
        MemoryType::PageLocked
    }
}

/// Safety
/// ------
/// - This works only when `dest` is host memory
#[allow(unused_unsafe)]
pub(super) unsafe fn copy_to_host<T: Copy>(
    dest: &mut impl MemoryMut<Elem = T>,
    src: &impl Memory<Elem = T>,
) {
    assert_ne!(dest.head_addr(), src.head_addr());
    assert_eq!(dest.byte_size(), src.byte_size());

    match src.memory_type() {
        // From host
        MemoryType::Host | MemoryType::Registered | MemoryType::PageLocked => dest
            .try_as_mut_slice()
            .unwrap()
            .copy_from_slice(src.try_as_slice().unwrap()),
        // From device
        MemoryType::Device => {
            let dest_ptr = dest.head_addr_mut();
            let src_ptr = src.head_addr();
            // context guard
            let _g = match (dest.try_get_context(), src.try_get_context()) {
                (Some(d_ctx), Some(s_ctx)) => {
                    assert_eq!(d_ctx, s_ctx);
                    Some(d_ctx.guard_context())
                }
                (Some(ctx), None) => Some(ctx.guard_context()),
                (None, Some(ctx)) => Some(ctx.guard_context()),
                (None, None) => None,
            };
            ffi_call!(
                cuMemcpyDtoH_v2,
                dest_ptr as _,
                src_ptr as _,
                dest.byte_size()
            )
            .expect("memcpy from Device to Host failed");
        }
        // From array
        MemoryType::Array => unimplemented!("Array memory is not supported yet"),
    }
}

impl<'ctx, T: Copy> MemoryMut for PageLockedMemory<'ctx, T> {
    fn head_addr_mut(&mut self) -> *mut T {
        self.ptr as _
    }
    fn try_as_mut_slice(&mut self) -> Result<&mut [T]> {
        Ok(self.as_mut_slice())
    }
    fn copy_from(&mut self, src: &impl Memory<Elem = Self::Elem>) {
        unsafe { copy_to_host(self, src) }
    }
}

impl<'ctx, T: Copy> Continuous for PageLockedMemory<'ctx, T> {
    fn length(&self) -> usize {
        self.size
    }
    fn as_slice(&self) -> &[T] {
        self
    }
}

impl<'ctx, T: Copy> ContinuousMut for PageLockedMemory<'ctx, T> {
    fn as_mut_slice(&mut self) -> &mut [T] {
        self
    }
}

impl<'ctx, T: Copy> Managed for PageLockedMemory<'ctx, T> {}

impl<'ctx, T> PageLockedMemory<'ctx, T> {
    /// Allocate host memory as page-locked.
    ///
    /// Allocating excessive amounts of pinned memory may degrade system performance,
    /// since it reduces the amount of memory available to the system for paging.
    /// As a result, this function is best used sparingly to allocate staging areas for data exchange between host and device.
    ///
    /// See also [cuMemAllocHost].
    ///
    /// [cuMemAllocHost]: https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__MEM.html#group__CUDA__MEM_1gdd8311286d2c2691605362c689bc64e0
    ///
    /// Panic
    /// ------
    /// - when memory allocation failed includeing `size == 0` case
    ///
    pub fn new(context: &'ctx Context, size: usize) -> Self {
        assert!(size > 0, "Zero-sized malloc is forbidden");
        let _g = context.guard_context();
        let ptr = ffi_new!(cuMemAllocHost_v2, size * std::mem::size_of::<T>())
            .expect("Cannot allocate page-locked memory");
        Self {
            ptr: ptr as *mut T,
            size,
            context,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::*;

    #[test]
    fn page_locked() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context();
        let mut mem = PageLockedMemory::<i32>::new(&ctx, 12);
        assert_eq!(mem.len(), 12);
        assert_eq!(mem.byte_size(), 12 * 4 /* size of i32 */);
        let sl = mem.as_mut_slice();
        sl[0] = 3;
        Ok(())
    }

    #[should_panic(expected = "Zero-sized malloc is forbidden")]
    #[test]
    fn page_locked_new_zero() {
        let device = Device::nth(0).unwrap();
        let ctx = device.create_context();
        let _a = PageLockedMemory::<i32>::new(&ctx, 0);
    }

    #[test]
    fn device() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context();
        let mut mem = DeviceMemory::<i32>::new(&ctx, 12);
        assert_eq!(mem.len(), 12);
        assert_eq!(mem.byte_size(), 12 * 4 /* size of i32 */);
        let sl = mem.as_mut_slice();
        sl[0] = 3;
        Ok(())
    }

    #[should_panic(expected = "Zero-sized malloc is forbidden")]
    #[test]
    fn device_new_zero() {
        let device = Device::nth(0).unwrap();
        let ctx = device.create_context();
        let _a = DeviceMemory::<i32>::new(&ctx, 0);
    }
}
