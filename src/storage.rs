use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use fnv::FnvHasher;

use bitset::BitSet;
use join::Join;
use world::{Component, Allocator};
use {Entity, Index, Generation};


/// The `UnprotectedStorage` together with the `BitSet` that knows
/// about which elements are stored, and which are not.
pub struct MaskedStorage<T: Component> {
    mask: BitSet,
    inner: T::Storage,
}

impl<T: Component> MaskedStorage<T> {
    /// Creates a new `MaskedStorage`. This is called when you register
    /// a new component type within the world.
    pub fn new() -> MaskedStorage<T> {
        MaskedStorage {
            mask: BitSet::new(),
            inner: UnprotectedStorage::new(),
        }
    }
    fn open_mut(&mut self) -> (&BitSet, &mut T::Storage) {
        (&self.mask, &mut self.inner)
    }
    /// Clear the contents of this storage.
    pub fn clear(&mut self) {
        let mask = &mut self.mask;
        unsafe {
            self.inner.clean(|i| mask.contains(i));
        }
        mask.clear();
    }
    /// Remove an element by a given index.
    pub fn remove(&mut self, id: Index) -> Option<T> {
        if self.mask.remove(id) {
            Some(unsafe { self.inner.remove(id) })
        }else {
            None
        }
    }
}

impl<T: Component> Drop for MaskedStorage<T> {
    fn drop(&mut self) {
        self.clear();
    }
}


/// A wrapper around the masked storage and the generations vector.
/// Can be used for safe lookup of components, insertions and removes.
/// This is what `World::read/write` locks for the user.
pub struct Storage<T, A, D> {
    phantom: PhantomData<T>,
    alloc: A,
    data: D,
}

impl<T, A, D> Storage<T, A, D> where
    A: Deref<Target = Allocator>,
{
    /// Create a new `Storage`
    pub fn new(alloc: A, data: D) -> Storage<T, A, D>{
        Storage {
            phantom: PhantomData,
            alloc: alloc,
            data: data,
        }
    }
    fn has_gen(&self, e: Entity) -> bool {
        let g1 = Generation(1);
        e.get_gen() == *self.alloc.generations.get(e.get_id() as usize).unwrap_or(&g1)
    }
}

impl<T, A, D> Storage<T, A, D> where
    T: Component,
    A: Deref<Target = Allocator>,
    D: Deref<Target = MaskedStorage<T>>,
{
    /// Tries to read the data associated with an `Entity`.
    pub fn get(&self, e: Entity) -> Option<&T> {
        if self.data.mask.contains(e.get_id()) && self.has_gen(e) {
            Some(unsafe { self.data.inner.get(e.get_id()) })
        }else {None}
    }
}

impl<T, A, D> Storage<T, A, D> where
    T: Component,
    A: Deref<Target = Allocator>,
    D: DerefMut<Target = MaskedStorage<T>>,
{
    /// Tries to mutate the data associated with an `Entity`.
    pub fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        if self.data.mask.contains(e.get_id()) && self.has_gen(e) {
            Some(unsafe { self.data.inner.get_mut(e.get_id()) })
        }else {None}
    }
    /// Inserts new data for a given `Entity`.
    /// Returns false if the entity is dead, and insertion is not possible.
    pub fn insert(&mut self, e: Entity, v: T) -> bool {
        if self.has_gen(e) {
            let id = e.get_id();
            if self.data.mask.contains(id) {
                *unsafe{ self.data.inner.get_mut(id) } = v;
            } else {
                self.data.mask.add(id);
                unsafe{ self.data.inner.insert(id, v) };
            }
            true
        }else {
            false
        }
    }
    /// Removes the data associated with an `Entity`.
    pub fn remove(&mut self, e: Entity) -> Option<T> {
        if self.has_gen(e) {
            self.data.remove(e.get_id())
        }else { None }
    }
    /// Clears the contents of the storage.
    pub fn clear(&mut self) {
        self.data.clear();
    }
}

impl<'a, T, A, D> Join for &'a Storage<T, A, D> where
    T: Component,
    A: Deref<Target = Allocator>,
    D: Deref<Target = MaskedStorage<T>>,
{
    type Type = &'a T;
    type Value = &'a T::Storage;
    type Mask = &'a BitSet;
    fn open(self) -> (Self::Mask, Self::Value) {
        (&self.data.mask, &self.data.inner)
    }
    unsafe fn get(v: &mut Self::Value, i: Index) -> &'a T {
        v.get(i)
    }
}

impl<'a, T, A, D> Join for &'a mut Storage<T, A, D> where
    T: Component,
    A: Deref<Target = Allocator>,
    D: DerefMut<Target = MaskedStorage<T>>,
{
    type Type = &'a mut T;
    type Value = &'a mut T::Storage;
    type Mask = &'a BitSet;
    fn open(self) -> (Self::Mask, Self::Value) {
        self.data.open_mut()
    }
    unsafe fn get(v: &mut Self::Value, i: Index) -> &'a mut T {
        use std::mem;
        // This is horribly unsafe. Unfortunately, Rust doesn't provide a way
        // to abstract mutable/immutable state at the moment, so we have to hack
        // our way through it.
        let value: &'a mut Self::Value = mem::transmute(v);
        value.get_mut(i)
    }
}


/// Used by the framework to quickly join componets
pub trait UnprotectedStorage<T>: Sized {
    /// Creates a new `Storage<T>`. This is called when you register a new
    /// component type within the world.
    fn new() -> Self;
    /// Clean the storage given a check to figure out if an index
    /// is valid or not. Allows us to safely drop the storage.
    unsafe fn clean<F>(&mut self, F) where F: Fn(Index) -> bool;
    /// Tries reading the data associated with an `Index`.
    /// This is unsafe because the external set used
    /// to protect this storage is absent.
    unsafe fn get(&self, id: Index) -> &T;
    /// Tries mutating the data associated with an `Index`.
    /// This is unsafe because the external set used
    /// to protect this storage is absent.
    unsafe fn get_mut(&mut self, id: Index) -> &mut T;
    /// Inserts new data for a given `Index`.
    unsafe fn insert(&mut self, Index, T);
    /// Removes the data associated with an `Index`.
    unsafe fn remove(&mut self, Index) -> T;
}

/// HashMap-based storage. Best suited for rare components.
pub struct HashMapStorage<T>(HashMap<Index, T, BuildHasherDefault<FnvHasher>>);

impl<T> UnprotectedStorage<T> for HashMapStorage<T> {
    fn new() -> Self {
        let fnv = BuildHasherDefault::<FnvHasher>::default();
        HashMapStorage(HashMap::with_hasher(fnv))
    }
    unsafe fn clean<F>(&mut self, _: F) where F: Fn(Index) -> bool {
        //nothing to do
    }
    unsafe fn get(&self, id: Index) -> &T {
        self.0.get(&id).unwrap()
    }
    unsafe fn get_mut(&mut self, id: Index) -> &mut T {
        self.0.get_mut(&id).unwrap()
    }
    unsafe fn insert(&mut self, id: Index, v: T) {
        self.0.insert(id, v);
    }
    unsafe fn remove(&mut self, id: Index) -> T {
        self.0.remove(&id).unwrap()
    }
}

/// Vec-based storage, stores the generations of the data in
/// order to match with given entities. Supposed to have maximum
/// performance for the components mostly present in entities.
pub struct VecStorage<T>(Vec<T>);

impl<T> UnprotectedStorage<T> for VecStorage<T> {
    fn new() -> Self {
        VecStorage(Vec::new())
    }
    unsafe fn clean<F>(&mut self, has: F) where F: Fn(Index) -> bool {
        use std::mem;
        for (i, v) in self.0.drain(..).enumerate() {
            if !has(i as Index) {
                // if v was not in the set the data is invalid
                // and we must forget it instead of dropping it
                mem::forget(v);
            }
        }
    }
    unsafe fn get(&self, id: Index) -> &T {
        self.0.get_unchecked(id as usize)
    }
    unsafe fn get_mut(&mut self, id: Index) -> &mut T {
        self.0.get_unchecked_mut(id as usize)
    }
    unsafe fn insert(&mut self, id: Index, mut v: T) {
        use std::mem;
        let id = id as usize;
        if self.0.len() <= id {
            let delta = id + 1 - self.0.len();
            self.0.reserve(delta);
            self.0.set_len(id + 1);
        }
        // Can't just do `self.0[id] = v` since it would
        // drop the existing element in there, which
        // is undefined at this point.
        mem::swap(self.0.get_unchecked_mut(id), &mut v);
        mem::forget(v);
    }
    unsafe fn remove(&mut self, id: Index) -> T {
        use std::ptr;
        ptr::read(self.get(id))
    }
}

/// A dummy storage type, used for cases where the component
/// doesn't contain any data and instead works as a simple flag.
pub struct DummyStorage<T>(T);

impl<T: Clone + Default> UnprotectedStorage<T> for DummyStorage<T> {
    fn new() -> Self {
        DummyStorage(Default::default())
    }
    unsafe fn clean<F>(&mut self, _: F) where F: Fn(Index) -> bool {}
    unsafe fn get(&self, _: Index) -> &T { &self.0 }
    unsafe fn get_mut(&mut self, _: Index) -> &mut T { &mut self.0 }
    unsafe fn insert(&mut self, _: Index, _: T) {}
    unsafe fn remove(&mut self, _: Index) -> T { self.0.clone() }
}


#[cfg(test)]
mod map_test {
    use mopa::Any;
    use super::{Storage, MaskedStorage, UnprotectedStorage, VecStorage};
    use world::Allocator;
    use {Component, Entity, Index, Generation};

    struct Comp<T>(T);
    impl<T: Any + Send + Sync> Component for Comp<T> {
        type Storage = VecStorage<Comp<T>>;
    }

    fn ent(i: Index) -> Entity {
        Entity::new(i, Generation(1))
    }

    #[test]
    fn insert() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        for i in 0..1_000 {
            c.insert(ent(i), Comp(i));
        }

        for i in 0..1_000 {
            assert_eq!(c.get(ent(i)).unwrap().0, i);
        }
    }

    #[test]
    fn insert_100k() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        for i in 0..100_000 {
            c.insert(ent(i), Comp(i));
        }

        for i in 0..100_000 {
            assert_eq!(c.get(ent(i)).unwrap().0, i);
        }
    }

    #[test]
    fn remove() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        for i in 0..1_000 {
            c.insert(ent(i), Comp(i));
        }

        for i in 0..1_000 {
            assert_eq!(c.get(ent(i)).unwrap().0, i);
        }

        for i in 0..1_000 {
            c.remove(ent(i));
        }

        for i in 0..1_000 {
            assert!(c.get(ent(i)).is_none());
        }
    }

    #[test]
    fn test_gen() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        for i in 0..1_000i32 {
            c.insert(ent(i as u32), Comp(i));
            c.insert(ent(i as u32), Comp(-i));
        }

        for i in 0..1_000i32 {
            assert_eq!(c.get(ent(i as u32)).unwrap().0, -i);
        }
    }

    #[test]
    fn insert_same_key() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        for i in 0..10_000 {
            c.insert(ent(i), Comp(i));
            assert_eq!(c.get(ent(i)).unwrap().0, i);
        }
    }

    #[should_panic]
    #[test]
    fn wrap() {
        let mut c = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::new()));

        c.insert(ent(1 << 25), Comp(7));
    }
}


#[cfg(test)]
mod test {
    use std::convert::AsMut;
    use std::fmt::Debug;
    use super::{Storage, MaskedStorage, VecStorage, HashMapStorage, DummyStorage};
    use world::Allocator;
    use {Component, Entity, Generation};

    #[derive(PartialEq, Eq, Debug)]
    struct Cvec(u32);
    impl From<u32> for Cvec {
        fn from(v: u32) -> Cvec { Cvec(v) }
    }
    impl AsMut<u32> for Cvec {
        fn as_mut(&mut self) -> &mut u32 { &mut self.0 }
    }
    impl Component for Cvec {
        type Storage = VecStorage<Cvec>;
    }

    #[derive(PartialEq, Eq, Debug)]
    struct Cmap(u32);
    impl From<u32> for Cmap {
        fn from(v: u32) -> Cmap { Cmap(v) }
    }
    impl AsMut<u32> for Cmap {
        fn as_mut(&mut self) -> &mut u32 { &mut self.0 }
    }
    impl Component for Cmap {
        type Storage = HashMapStorage<Cmap>;
    }

    #[derive(Clone)]
    struct Cdummy(u32);
    impl Default for Cdummy {
        fn default() -> Cdummy { Cdummy(0) }
    }
    impl From<u32> for Cdummy {
        fn from(v: u32) -> Cdummy { Cdummy(v) }
    }
    impl Component for Cdummy {
        type Storage = DummyStorage<Cdummy>;
    }

    fn test_add<T: Component + From<u32> + Debug + Eq>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..1_000 {
            s.insert(Entity::new(i, Generation(1)), (i + 2718).into());
        }

        for i in 0..1_000 {
            assert_eq!(s.get(Entity::new(i, Generation(1))).unwrap(), &(i + 2718).into());
        }
    }

    fn test_sub<T: Component + From<u32> + Debug + Eq>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..1_000 {
            s.insert(Entity::new(i, Generation(1)), (i + 2718).into());
        }

        for i in 0..1_000 {
            assert_eq!(s.remove(Entity::new(i, Generation(1))).unwrap(), (i + 2718).into());
            assert!(s.remove(Entity::new(i, Generation(1))).is_none());
        }
    }

    fn test_get_mut<T: Component + From<u32> + AsMut<u32> + Debug + Eq>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..1_000 {
            s.insert(Entity::new(i, Generation(1)), (i + 2718).into());
        }

        for i in 0..1_000 {
            *s.get_mut(Entity::new(i, Generation(1))).unwrap().as_mut() -= 718;
        }

        for i in 0..1_000 {
            assert_eq!(s.get(Entity::new(i, Generation(1))).unwrap(), &(i + 2000).into());
        }
    }

    fn test_add_gen<T: Component + From<u32> + Debug + Eq>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..1_000 {
            s.insert(Entity::new(i, Generation(1)), (i + 2718).into());
            s.insert(Entity::new(i, Generation(2)), (i + 31415).into());
        }

        for i in 0..1_000 {
            assert!(s.get(Entity::new(i, Generation(2))).is_none());
            assert_eq!(s.get(Entity::new(i, Generation(1))).unwrap(), &(i + 2718).into());
        }
    }

    fn test_sub_gen<T: Component + From<u32> + Debug + Eq>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..1_000 {
            s.insert(Entity::new(i, Generation(2)), (i + 2718).into());
        }

        for i in 0..1_000 {
            assert!(s.remove(Entity::new(i, Generation(1))).is_none());
        }
    }

    fn test_clear<T: Component + From<u32>>() {
        let mut s = Storage::new(Box::new(Allocator::new()), Box::new(MaskedStorage::<T>::new()));

        for i in 0..10 {
            s.insert(Entity::new(i, Generation(1)), (i + 10).into());
        }

        s.clear();

        for i in 0..10 {
            assert!(s.get(Entity::new(i, Generation(1))).is_none());
        }
    }


    #[test] fn vec_test_add() { test_add::<Cvec>(); }
    #[test] fn vec_test_sub() { test_sub::<Cvec>(); }
    #[test] fn vec_test_get_mut() { test_get_mut::<Cvec>(); }
    #[test] fn vec_test_add_gen() { test_add_gen::<Cvec>(); }
    #[test] fn vec_test_sub_gen() { test_sub_gen::<Cvec>(); }
    #[test] fn vec_test_clear() { test_clear::<Cvec>(); }

    #[test] fn hash_test_add() { test_add::<Cmap>(); }
    #[test] fn hash_test_sub() { test_sub::<Cmap>(); }
    #[test] fn hash_test_get_mut() { test_get_mut::<Cmap>(); }
    #[test] fn hash_test_add_gen() { test_add_gen::<Cmap>(); }
    #[test] fn hash_test_sub_gen() { test_sub_gen::<Cmap>(); }
    #[test] fn hash_test_clear() { test_clear::<Cmap>(); }

    #[test] fn dummy_test_clear() { test_clear::<Cdummy>(); }
}

