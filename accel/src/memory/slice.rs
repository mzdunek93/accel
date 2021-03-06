use super::*;

/// Typed wrapper of cuPointerGetAttribute
fn get_attr<T, Attr>(ptr: *const T, attr: CUpointer_attribute) -> error::Result<Attr> {
    let mut data = MaybeUninit::<Attr>::uninit();
    unsafe {
        ffi_call!(
            cuPointerGetAttribute,
            data.as_mut_ptr() as *mut c_void,
            attr,
            ptr as CUdeviceptr
        )?;
        Ok(data.assume_init())
    }
}

/// Determine actual memory type dynamically
///
/// Because `Continuous` memories can be treated as a slice,
/// input slice may represents any type of memory.
fn memory_type<T>(ptr: *const T) -> MemoryType {
    match get_attr(ptr, CUpointer_attribute::CU_POINTER_ATTRIBUTE_MEMORY_TYPE) {
        Ok(CUmemorytype_enum::CU_MEMORYTYPE_HOST) => MemoryType::PageLocked,
        Ok(CUmemorytype_enum::CU_MEMORYTYPE_DEVICE) => MemoryType::Device,
        Ok(CUmemorytype_enum::CU_MEMORYTYPE_ARRAY) => MemoryType::Array,
        Ok(CUmemorytype_enum::CU_MEMORYTYPE_UNIFIED) => {
            unreachable!("CU_POINTER_ATTRIBUTE_MEMORY_TYPE never be UNIFED")
        }
        Err(_) => {
            // unmanaged by CUDA memory system, i.e. host memory
            MemoryType::Host
        }
    }
}

fn get_context<T>(ptr: *const T) -> Option<ContextRef> {
    let ptr =
        get_attr::<_, CUcontext>(ptr, CUpointer_attribute::CU_POINTER_ATTRIBUTE_CONTEXT).ok()?;
    Some(ContextRef::from_ptr(ptr))
}

impl<T: Scalar> Memory for [T] {
    type Elem = T;
    fn head_addr(&self) -> *const T {
        self.as_ptr()
    }

    fn head_addr_mut(&mut self) -> *mut T {
        self.as_mut_ptr()
    }

    fn num_elem(&self) -> usize {
        self.len()
    }

    fn memory_type(&self) -> MemoryType {
        memory_type(self.as_ptr())
    }
}

impl<T: Scalar> Memcpy<[T]> for [T] {
    fn copy_from(&mut self, src: &[T]) {
        assert_ne!(self.head_addr(), src.head_addr());
        assert_eq!(self.num_elem(), src.num_elem());
        if let Some(ctx) = get_context(self.head_addr()).or_else(|| get_context(src.head_addr())) {
            unsafe {
                contexted_call!(
                    &ctx,
                    cuMemcpy,
                    self.head_addr_mut() as CUdeviceptr,
                    src.as_ptr() as CUdeviceptr,
                    self.num_elem() * T::size_of()
                )
            }
            .unwrap()
        } else {
            self.copy_from_slice(src);
        }
    }
}

macro_rules! impl_memcpy_slice {
    ($t:path) => {
        impl<T: Scalar> Memcpy<[T]> for $t {
            fn copy_from(&mut self, src: &[T]) {
                self.as_mut_slice().copy_from(src);
            }
        }
        impl<T: Scalar> Memcpy<$t> for [T] {
            fn copy_from(&mut self, src: &$t) {
                self.copy_from(src.as_slice());
            }
        }
    };
}

impl_memcpy_slice!(DeviceMemory::<T>);
impl_memcpy_slice!(PageLockedMemory::<T>);
impl_memcpy_slice!(RegisteredMemory::<'_, T>);

macro_rules! impl_memcpy {
    ($from:path, $to:path) => {
        impl<T: Scalar> Memcpy<$from> for $to {
            fn copy_from(&mut self, src: &$from) {
                self.as_mut_slice().copy_from(src.as_slice());
            }
        }
    };
}

impl_memcpy!(DeviceMemory::<T>, DeviceMemory::<T>);
impl_memcpy!(DeviceMemory::<T>, RegisteredMemory::<'_, T>);
impl_memcpy!(DeviceMemory::<T>, PageLockedMemory::<T>);
impl_memcpy!(PageLockedMemory::<T>, DeviceMemory::<T>);
impl_memcpy!(PageLockedMemory::<T>, RegisteredMemory::<'_, T>);
impl_memcpy!(PageLockedMemory::<T>, PageLockedMemory::<T>);
impl_memcpy!(RegisteredMemory::<'_, T>, DeviceMemory::<T>);
impl_memcpy!(RegisteredMemory::<'_, T>, RegisteredMemory::<'_, T>);
impl_memcpy!(RegisteredMemory::<'_, T>, PageLockedMemory::<T>);

impl<T: Scalar> Continuous for [T] {
    fn as_slice(&self) -> &[Self::Elem] {
        self
    }

    fn as_mut_slice(&mut self) -> &mut [Self::Elem] {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_host_vec() -> error::Result<()> {
        let a = vec![0_u32; 12];
        assert_eq!(a.as_slice().memory_type(), MemoryType::Host);
        assert_eq!(a.as_slice().num_elem(), 12);
        Ok(())
    }

    #[test]
    fn memory_type_host_vec_with_context() -> error::Result<()> {
        let device = Device::nth(0)?;
        let _ctx = device.create_context();
        let a = vec![0_u32; 12];
        assert_eq!(a.as_slice().memory_type(), MemoryType::Host);
        assert_eq!(a.as_slice().num_elem(), 12);
        Ok(())
    }

    #[test]
    fn restore_context() -> error::Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context();
        let a = PageLockedMemory::<i32>::zeros(&ctx, 12);
        let ctx_ptr = get_context(a.head_addr()).unwrap();
        assert_eq!(*ctx, ctx_ptr);
        Ok(())
    }
}
