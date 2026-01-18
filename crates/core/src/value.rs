//! Based on <https://github.com/dtolnay/erased-serde/blob/650ab83dcff1908d6f1c5e902a5cac1277d51dee/src/any.rs>

use std::any::TypeId;
use std::marker::PhantomData;
use std::mem::{self, MaybeUninit};
use std::ptr;

use erased_serde::{self, Serialize as ErasedSerialize};
use serde::{Serialize, ser};
struct ValueDispatch<T>(PhantomData<T>);

impl<T: Serialize + Clone + Send + Sync + 'static> ValueDispatch<T> {
    /// SAFETY: Caller must ensure that `value` actually contains a `T` – this is
    /// guaranteed by `Value::new` which records the `type_id`.
    #[inline]
    unsafe fn get(value: &ValueInner) -> &T {
        if is_small::<T>() {
            unsafe { &*value.inline.as_ptr().cast::<T>() }
        } else {
            unsafe { &*value.ptr.cast::<T>() }
        }
    }

    unsafe fn inline_drop(value: &mut ValueInner) {
        unsafe { ptr::drop_in_place(value.inline.as_mut_ptr().cast::<T>()) }
    }

    unsafe fn inline_clone(value: &ValueInner) -> Value {
        let inner = unsafe { Self::get(value) };
        let cloned = inner.clone();
        Value::new(cloned)
    }

    unsafe fn inline_serialize(
        value: &ValueInner,
        serializer: &mut dyn erased_serde::Serializer,
    ) -> erased_serde::Result<()> {
        let inner = unsafe { Self::get(value) };
        inner.erased_serialize(serializer)
    }

    unsafe fn ptr_drop(value: &mut ValueInner) {
        mem::drop(unsafe { Box::from_raw(value.ptr.cast::<T>()) });
    }

    unsafe fn ptr_clone(value: &ValueInner) -> Value {
        let inner = unsafe { Self::get(value) };
        let cloned = inner.clone();
        Value::new(cloned)
    }

    unsafe fn ptr_serialize(
        value: &ValueInner,
        serializer: &mut dyn erased_serde::Serializer,
    ) -> erased_serde::Result<()> {
        let inner = unsafe { Self::get(value) };
        inner.erased_serialize(serializer)
    }
}

pub struct ValueVTable {
    drop: unsafe fn(&mut ValueInner),
    clone: unsafe fn(&ValueInner) -> Value,
    serialize:
        unsafe fn(&ValueInner, &mut dyn erased_serde::Serializer) -> erased_serde::Result<()>,
}

impl ValueVTable {
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>() -> Self {
        if is_small::<T>() {
            Self {
                drop: ValueDispatch::<T>::inline_drop,
                clone: ValueDispatch::<T>::inline_clone,
                serialize: ValueDispatch::<T>::inline_serialize,
            }
        } else {
            Self {
                drop: ValueDispatch::<T>::ptr_drop,
                clone: ValueDispatch::<T>::ptr_clone,
                serialize: ValueDispatch::<T>::ptr_serialize,
            }
        }
    }
}

pub struct Value {
    value: ValueInner,
    vtable: ValueVTable,
    type_id: TypeId,
}

unsafe impl Send for Value {}
unsafe impl Sync for Value {}

union ValueInner {
    ptr: *mut (),
    inline: [MaybeUninit<usize>; 2],
}

const fn is_small<T>() -> bool {
    mem::size_of::<T>() <= mem::size_of::<ValueInner>()
        && mem::align_of::<T>() <= mem::align_of::<ValueInner>()
}

impl Value {
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>(t: T) -> Self {
        let type_id = TypeId::of::<T>();
        let vtable = ValueVTable::new::<T>();

        if is_small::<T>() {
            let mut inline = [MaybeUninit::uninit(); 2];
            unsafe { ptr::write(inline.as_mut_ptr().cast::<T>(), t) };
            let value = ValueInner { inline };

            Value {
                value,
                vtable,
                type_id,
            }
        } else {
            let ptr = Box::into_raw(Box::new(t)).cast::<()>();
            let value = ValueInner { ptr };

            Value {
                value,
                vtable,
                type_id,
            }
        }
    }

    pub fn take<T: 'static>(self) -> Option<T> {
        if self.type_id == TypeId::of::<T>() {
            // SAFETY: The `type_id` is guaranteed to match the type stored in
            // `self.value` by the `Value::new` constructor.
            Some(unsafe { self.take_unchecked::<T>() })
        } else {
            None
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that `self` actually contains a `T`
    pub unsafe fn take_unchecked<T: 'static>(mut self) -> T {
        debug_assert_eq!(self.type_id, TypeId::of::<T>(), "invalid cast");

        if is_small::<T>() {
            let ptr = unsafe { self.value.inline.as_mut_ptr().cast::<T>() };
            let value = unsafe { ptr::read(ptr) };
            mem::forget(self);
            value
        } else {
            let ptr = unsafe { self.value.ptr.cast::<T>() };
            let box_t = unsafe { Box::from_raw(ptr) };
            mem::forget(self);
            *box_t
        }
    }

    pub fn serialize<S>(&self, serializer: S) -> Result<(), S::Error>
    where
        S: serde::Serializer,
    {
        let mut erased = <dyn erased_serde::Serializer>::erase(serializer);
        // SAFETY: The `serialize` function pointer is guaranteed to match the
        // type stored in `self.value` by the `Value::new` constructor.
        unsafe { (self.vtable.serialize)(&self.value, &mut erased) }.map_err(ser::Error::custom)
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        if self.type_id == TypeId::of::<T>() {
            // SAFETY: The `type_id` is guaranteed to match the type stored in
            // `self.value` by the `Value::new` constructor.
            Some(unsafe { self.downcast_ref_unchecked::<T>() })
        } else {
            None
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that `self` actually contains a `T`
    pub unsafe fn downcast_ref_unchecked<T: 'static>(&self) -> &T {
        debug_assert_eq!(self.type_id, TypeId::of::<T>(), "invalid cast");

        let ptr = if is_small::<T>() {
            unsafe { self.value.inline.as_ptr().cast::<T>() }
        } else {
            unsafe { self.value.ptr.cast::<T>() }
        };
        unsafe { &*ptr }
    }

    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        if self.type_id == TypeId::of::<T>() {
            // SAFETY: The `type_id` is guaranteed to match the type stored in
            // `self.value` by the `Value::new` constructor.
            Some(unsafe { self.downcast_mut_unchecked::<T>() })
        } else {
            None
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that `self` actually contains a `T`
    pub unsafe fn downcast_mut_unchecked<T: 'static>(&mut self) -> &mut T {
        debug_assert_eq!(self.type_id, TypeId::of::<T>(), "invalid cast");

        let ptr = if is_small::<T>() {
            unsafe { self.value.inline.as_mut_ptr().cast::<T>() }
        } else {
            unsafe { self.value.ptr.cast::<T>() }
        };
        unsafe { &mut *ptr }
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        unsafe { (self.vtable.clone)(&self.value) }
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        unsafe { (self.vtable.drop)(&mut self.value) }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Value").finish()
    }
}
