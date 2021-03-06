use std::alloc;
use std::ops::{Deref, DerefMut};
use std::slice::from_raw_parts;
use std::{marker::PhantomData, ptr::NonNull};

#[derive(Debug)]
struct RawVec<T> {
    /// 1. NonNull<T> will never be Null
    /// 2. NonNull<T> is covariant over T
    ptr: NonNull<T>,
    cap: usize,
    /// Pretending to own T for dropck later
    _marker: PhantomData<T>,
}
unsafe impl<T: Sync> Sync for RawVec<T> {}
unsafe impl<T: Send> Send for RawVec<T> {}

impl<T> RawVec<T> {
    fn new() -> Self {
        // if the size of T is 0, we set the capacity to be i32::MAX
        let cap = if std::mem::size_of::<T>() == 0 { !0 } else { 0 };
        RawVec {
            ptr: NonNull::dangling(),
            cap,
            _marker: PhantomData,
        }
    }

    fn grow(&mut self) {
        assert!(std::mem::size_of::<T>() != 0, "capacity overflow");
        let new_cap = if self.cap == 0 { 1 } else { self.cap * 2 };
        let new_layout = alloc::Layout::array::<T>(new_cap).unwrap();

        // ptr::offset takes an `isize` parameter which is the max number of units of T a pointer
        // can possibly reach
        assert!(
            new_layout.size() <= isize::MAX as usize,
            "Allocation too large"
        );

        let new_ptr = if self.cap == 0 {
            unsafe { alloc::alloc(new_layout) }
        } else {
            let old_layout = alloc::Layout::array::<T>(self.cap).unwrap();
            let old_ptr = self.ptr.as_ptr() as *mut u8;
            unsafe { alloc::realloc(old_ptr, old_layout, new_cap) }
        };

        // if allocation failed, None will be returned
        self.ptr = match NonNull::new(new_ptr as *mut T) {
            Some(p) => p,
            None => {
                alloc::handle_alloc_error(new_layout);
            }
        };
        self.cap = new_cap;
    }
}

impl<T> Drop for RawVec<T> {
    fn drop(&mut self) {
        if self.cap != 0 && std::mem::size_of::<T>() != 0 {
            let layout = alloc::Layout::array::<T>(self.cap).unwrap();
            unsafe {
                alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}


#[derive(Debug)]
pub struct MyVec<T> {
    buf: RawVec<T>,
    len: usize,
}

impl<T> MyVec<T> {
    pub fn new() -> Self {
        Self {
            buf: RawVec::new(),
            len: 0,
        }
    }

    fn cap(&self) -> usize {
        self.buf.cap
    }

    fn ptr(&self) -> *mut T {
        self.buf.ptr.as_ptr()
    }

    pub fn push(&mut self, ele: T) {
        if self.len == self.cap() {
            self.buf.grow();
        }
        unsafe {
            std::ptr::write(self.ptr().add(self.len), ele);
        }

        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            unsafe { Some(std::ptr::read(self.ptr().add(self.len))) }
        }
    }

    pub fn insert(&mut self, idx: usize, ele: T) {
        assert!(idx <= self.len, "index out of bounds");
        if self.len == self.cap() {
            self.buf.grow();
        }
        unsafe {
            let ptr = self.ptr().add(idx);
            let new_ptr = self.ptr().add(idx + 1);
            let count = self.len - idx;
            std::ptr::copy(ptr, new_ptr, count);
            std::ptr::write(ptr, ele);
            self.len += 1;
        }
    }

    pub fn remove(&mut self, idx: usize) -> T {
        assert!(idx < self.len, "index out of bounds");
        unsafe {
            let ptr = self.ptr().add(idx + 1);
            let new_ptr = self.ptr().add(idx);
            let item = new_ptr.read();
            let count = self.len - idx - 1;
            std::ptr::copy(ptr, new_ptr, count);
            self.len -= 1;
            return item;
        }
    }

    pub fn into_iter(self) -> IntoIter<T> {
        unsafe {
            let buf = std::ptr::read(&self.buf);
            let iter = RawValIter::new(&self);

            std::mem::forget(self);
            // take ownership of self without running its destructor
            IntoIter {
                iter,
                _buf: buf,
            }
        }
    }

    pub fn drain(&mut self) -> Drain<T> {
        unsafe {
            let iter = RawValIter::new(&self);
            // std::mem::forget(self);
            self.len = 0;
            Drain {
                iter,
                vec: PhantomData
            }
        }
    }
}

impl<T> Deref for MyVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe { std::slice::from_raw_parts(self.ptr(), self.len) }
    }
}

impl<T> DerefMut for MyVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::slice::from_raw_parts_mut(self.ptr(), self.len) }
    }
}

impl<T> Drop for MyVec<T> {
    fn drop(&mut self) {
        // this could be removed when T:!Drop as in the elements don't need to be dropped
        while let Some(_) = self.pop() {}
    }
}


pub struct RawValIter<T> {
    start: *const T,
    end: *const T
}

impl<T> RawValIter<T> {
    unsafe fn new(slice:&[T]) -> Self {
        Self {
            start: slice.as_ptr(),
            end: if std::mem::size_of::<T>() == 0 {
                ((slice.as_ptr() as usize) + slice.len()) as *const _
            } else if slice.len() == 0 {
                slice.as_ptr()
            } else {
                slice.as_ptr().add(slice.len())
            }
        }
    }
}

impl<T> Iterator for RawValIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                let result = std::ptr::read(self.start);
                self.start = if std::mem::size_of::<T>() == 0 {
                    ((self.start as usize) + 1) as *const _
                } else {
                    self.start.add(1)
                };
                Some(result)
            }
        }
    }
}

impl<T> DoubleEndedIterator for RawValIter<T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                self.end = if std::mem::size_of::<T>() == 0 {
                    ((self.end as usize) - 1 ) as *const _
                } else {
                    self.end.sub(1)
                };
                Some(std::ptr::read(self.end))
            }
        }
    }
}

pub struct Drain<'a,T> {
    vec: PhantomData<&'a mut MyVec<T>>,
    iter: RawValIter<T>
}

impl<'a, T> Iterator for Drain<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl<'a, T> DoubleEndedIterator for Drain<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back()
    }
}

impl<'a, T> Drop for Drain<'a, T> {
    fn drop(&mut self) {
        for _ in &mut *self { }
    }
}

pub struct IntoIter<T> {
    _buf: RawVec<T>,
    iter: RawValIter<T>
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> { self.iter.next() }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<Self::Item> { self.iter.next_back() }
}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        for _ in &mut *self {}
    }
}

#[cfg(test)]
mod tests {
    use super::MyVec;
    #[test]
    fn create_new_success() {
        let v: MyVec<i32> = MyVec::new();
        assert!(std::mem::size_of_val(&v) != 0);
    }
    #[test]
    #[should_panic]
    fn create_new_fail() {
        let v: MyVec<()> = MyVec::new();
        assert!(std::mem::size_of_val(&v) == 0);
    }

    #[test]
    fn push() {
        let mut v: MyVec<i32> = MyVec::new();
        assert_eq!(v.len(), 0);
        v.push(1);
        assert_eq!(v.len(), 1);
        v.push(2);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn pop() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        assert_eq!(v.pop(), Some(2));
        assert_eq!(v.pop(), Some(1));
        assert_eq!(v.pop(), None);
    }

    #[test]
    fn deref_to_slice_inbound() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        assert_eq!(v[2], 3);
    }

    #[test]
    #[should_panic]
    fn deref_to_slice_outbound() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        assert_eq!(v[2], 3);
    }

    #[test]
    fn deref_mut_inbound() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v[1] = 4;
        assert_eq!(v[2], 3);
    }

    #[test]
    #[should_panic]
    fn deref_mut_outbound() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v[4] = 4;
    }

    #[test]
    fn test_insert() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v.insert(2, 9);
        assert_eq!(v[3], 3);
        assert_eq!(v[2], 9);
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn test_remove_success() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        assert_eq!(v.remove(0), 1);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], 2);
    }

    #[should_panic]
    #[test]
    fn test_remove_fail() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.remove(2);
    }

    #[test]
    fn test_into_iter() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v.push(4);
        v.push(5);
        let mut it = v.into_iter();
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(2));
        assert_eq!(it.next_back(), Some(5));
        assert_eq!(it.next_back(), Some(4));
        assert_eq!(it.next(), Some(3));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
    }

    #[test]
    fn test_drain() {
        let mut v: MyVec<i32> = MyVec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        v.push(4);
        v.push(5);
        let mut it = v.drain();
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(2));
        assert_eq!(it.next_back(), Some(5));
        assert_eq!(it.next_back(), Some(4));
        assert_eq!(it.next(), Some(3));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
    }

    #[test]
    fn test_zst_drain() {
        let mut v: MyVec<()> = MyVec::new();
        v.push(());
        v.push(());
        v.push(());
        v.push(());
        v.push(());
        let mut it = v.drain();
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next_back(), Some(()));
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next_back(), Some(()));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
    }

    #[test]
    fn test_zst_iter() {
        let mut v: MyVec<()> = MyVec::new();
        v.push(());
        v.push(());
        v.push(());
        v.push(());
        v.push(());
        let mut it = v.into_iter();
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next_back(), Some(()));
        assert_eq!(it.next(), Some(()));
        assert_eq!(it.next_back(), Some(()));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
    }
}
