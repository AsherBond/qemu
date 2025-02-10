// Copyright 2024, Linaro Limited
// Author(s): Manos Pitsidianakis <manos.pitsidianakis@linaro.org>
// SPDX-License-Identifier: GPL-2.0-or-later

//! Bindings to create devices and access device functionality from Rust.

use std::{
    ffi::{CStr, CString},
    os::raw::c_void,
    ptr::NonNull,
};

pub use bindings::{Clock, ClockEvent, DeviceClass, DeviceState, Property, ResetType};

use crate::{
    bindings::{self, Error, ResettableClass},
    callbacks::FnCall,
    cell::bql_locked,
    chardev::Chardev,
    prelude::*,
    qom::{ClassInitImpl, ObjectClass, ObjectImpl, Owned},
    vmstate::VMStateDescription,
};

/// Trait providing the contents of the `ResettablePhases` struct,
/// which is part of the QOM `Resettable` interface.
pub trait ResettablePhasesImpl {
    /// If not None, this is called when the object enters reset. It
    /// can reset local state of the object, but it must not do anything that
    /// has a side-effect on other objects, such as raising or lowering an
    /// [`InterruptSource`](crate::irq::InterruptSource), or reading or
    /// writing guest memory. It takes the reset's type as argument.
    const ENTER: Option<fn(&Self, ResetType)> = None;

    /// If not None, this is called when the object for entry into reset, once
    /// every object in the system which is being reset has had its
    /// `ResettablePhasesImpl::ENTER` method called. At this point devices
    /// can do actions that affect other objects.
    ///
    /// If in doubt, implement this method.
    const HOLD: Option<fn(&Self, ResetType)> = None;

    /// If not None, this phase is called when the object leaves the reset
    /// state. Actions affecting other objects are permitted.
    const EXIT: Option<fn(&Self, ResetType)> = None;
}

/// # Safety
///
/// We expect the FFI user of this function to pass a valid pointer that
/// can be downcasted to type `T`. We also expect the device is
/// readable/writeable from one thread at any time.
unsafe extern "C" fn rust_resettable_enter_fn<T: ResettablePhasesImpl>(
    obj: *mut Object,
    typ: ResetType,
) {
    let state = NonNull::new(obj).unwrap().cast::<T>();
    T::ENTER.unwrap()(unsafe { state.as_ref() }, typ);
}

/// # Safety
///
/// We expect the FFI user of this function to pass a valid pointer that
/// can be downcasted to type `T`. We also expect the device is
/// readable/writeable from one thread at any time.
unsafe extern "C" fn rust_resettable_hold_fn<T: ResettablePhasesImpl>(
    obj: *mut Object,
    typ: ResetType,
) {
    let state = NonNull::new(obj).unwrap().cast::<T>();
    T::HOLD.unwrap()(unsafe { state.as_ref() }, typ);
}

/// # Safety
///
/// We expect the FFI user of this function to pass a valid pointer that
/// can be downcasted to type `T`. We also expect the device is
/// readable/writeable from one thread at any time.
unsafe extern "C" fn rust_resettable_exit_fn<T: ResettablePhasesImpl>(
    obj: *mut Object,
    typ: ResetType,
) {
    let state = NonNull::new(obj).unwrap().cast::<T>();
    T::EXIT.unwrap()(unsafe { state.as_ref() }, typ);
}

/// Trait providing the contents of [`DeviceClass`].
pub trait DeviceImpl: ObjectImpl + ResettablePhasesImpl {
    /// _Realization_ is the second stage of device creation. It contains
    /// all operations that depend on device properties and can fail (note:
    /// this is not yet supported for Rust devices).
    ///
    /// If not `None`, the parent class's `realize` method is overridden
    /// with the function pointed to by `REALIZE`.
    const REALIZE: Option<fn(&Self)> = None;

    /// An array providing the properties that the user can set on the
    /// device.  Not a `const` because referencing statics in constants
    /// is unstable until Rust 1.83.0.
    fn properties() -> &'static [Property] {
        &[]
    }

    /// A `VMStateDescription` providing the migration format for the device
    /// Not a `const` because referencing statics in constants is unstable
    /// until Rust 1.83.0.
    fn vmsd() -> Option<&'static VMStateDescription> {
        None
    }
}

/// # Safety
///
/// This function is only called through the QOM machinery and
/// used by the `ClassInitImpl<DeviceClass>` trait.
/// We expect the FFI user of this function to pass a valid pointer that
/// can be downcasted to type `T`. We also expect the device is
/// readable/writeable from one thread at any time.
unsafe extern "C" fn rust_realize_fn<T: DeviceImpl>(dev: *mut DeviceState, _errp: *mut *mut Error) {
    let state = NonNull::new(dev).unwrap().cast::<T>();
    T::REALIZE.unwrap()(unsafe { state.as_ref() });
}

unsafe impl InterfaceType for ResettableClass {
    const TYPE_NAME: &'static CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(bindings::TYPE_RESETTABLE_INTERFACE) };
}

impl<T> ClassInitImpl<ResettableClass> for T
where
    T: ResettablePhasesImpl,
{
    fn class_init(rc: &mut ResettableClass) {
        if <T as ResettablePhasesImpl>::ENTER.is_some() {
            rc.phases.enter = Some(rust_resettable_enter_fn::<T>);
        }
        if <T as ResettablePhasesImpl>::HOLD.is_some() {
            rc.phases.hold = Some(rust_resettable_hold_fn::<T>);
        }
        if <T as ResettablePhasesImpl>::EXIT.is_some() {
            rc.phases.exit = Some(rust_resettable_exit_fn::<T>);
        }
    }
}

impl<T> ClassInitImpl<DeviceClass> for T
where
    T: ClassInitImpl<ObjectClass> + ClassInitImpl<ResettableClass> + DeviceImpl,
{
    fn class_init(dc: &mut DeviceClass) {
        if <T as DeviceImpl>::REALIZE.is_some() {
            dc.realize = Some(rust_realize_fn::<T>);
        }
        if let Some(vmsd) = <T as DeviceImpl>::vmsd() {
            dc.vmsd = vmsd;
        }
        let prop = <T as DeviceImpl>::properties();
        if !prop.is_empty() {
            unsafe {
                bindings::device_class_set_props_n(dc, prop.as_ptr(), prop.len());
            }
        }

        ResettableClass::interface_init::<T, DeviceState>(dc);
        <T as ClassInitImpl<ObjectClass>>::class_init(&mut dc.parent_class);
    }
}

#[macro_export]
macro_rules! define_property {
    ($name:expr, $state:ty, $field:ident, $prop:expr, $type:ty, bit = $bitnr:expr, default = $defval:expr$(,)*) => {
        $crate::bindings::Property {
            // use associated function syntax for type checking
            name: ::std::ffi::CStr::as_ptr($name),
            info: $prop,
            offset: $crate::offset_of!($state, $field) as isize,
            bitnr: $bitnr,
            set_default: true,
            defval: $crate::bindings::Property__bindgen_ty_1 { u: $defval as u64 },
            ..$crate::zeroable::Zeroable::ZERO
        }
    };
    ($name:expr, $state:ty, $field:ident, $prop:expr, $type:ty, default = $defval:expr$(,)*) => {
        $crate::bindings::Property {
            // use associated function syntax for type checking
            name: ::std::ffi::CStr::as_ptr($name),
            info: $prop,
            offset: $crate::offset_of!($state, $field) as isize,
            set_default: true,
            defval: $crate::bindings::Property__bindgen_ty_1 { u: $defval as u64 },
            ..$crate::zeroable::Zeroable::ZERO
        }
    };
    ($name:expr, $state:ty, $field:ident, $prop:expr, $type:ty$(,)*) => {
        $crate::bindings::Property {
            // use associated function syntax for type checking
            name: ::std::ffi::CStr::as_ptr($name),
            info: $prop,
            offset: $crate::offset_of!($state, $field) as isize,
            set_default: false,
            ..$crate::zeroable::Zeroable::ZERO
        }
    };
}

#[macro_export]
macro_rules! declare_properties {
    ($ident:ident, $($prop:expr),*$(,)*) => {
        pub static $ident: [$crate::bindings::Property; {
            let mut len = 0;
            $({
                _ = stringify!($prop);
                len += 1;
            })*
            len
        }] = [
            $($prop),*,
        ];
    };
}

unsafe impl ObjectType for DeviceState {
    type Class = DeviceClass;
    const TYPE_NAME: &'static CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(bindings::TYPE_DEVICE) };
}
qom_isa!(DeviceState: Object);

/// Trait for methods exposed by the [`DeviceState`] class.  The methods can be
/// called on all objects that have the trait `IsA<DeviceState>`.
///
/// The trait should only be used through the blanket implementation,
/// which guarantees safety via `IsA`.
pub trait DeviceMethods: ObjectDeref
where
    Self::Target: IsA<DeviceState>,
{
    /// Add an input clock named `name`.  Invoke the callback with
    /// `self` as the first parameter for the events that are requested.
    ///
    /// The resulting clock is added as a child of `self`, but it also
    /// stays alive until after `Drop::drop` is called because C code
    /// keeps an extra reference to it until `device_finalize()` calls
    /// `qdev_finalize_clocklist()`.  Therefore (unlike most cases in
    /// which Rust code has a reference to a child object) it would be
    /// possible for this function to return a `&Clock` too.
    #[inline]
    fn init_clock_in<F: for<'a> FnCall<(&'a Self::Target, ClockEvent)>>(
        &self,
        name: &str,
        _cb: &F,
        events: ClockEvent,
    ) -> Owned<Clock> {
        fn do_init_clock_in(
            dev: *mut DeviceState,
            name: &str,
            cb: Option<unsafe extern "C" fn(*mut c_void, ClockEvent)>,
            events: ClockEvent,
        ) -> Owned<Clock> {
            assert!(bql_locked());

            // SAFETY: the clock is heap allocated, but qdev_init_clock_in()
            // does not gift the reference to its caller; so use Owned::from to
            // add one.  The callback is disabled automatically when the clock
            // is unparented, which happens before the device is finalized.
            unsafe {
                let cstr = CString::new(name).unwrap();
                let clk = bindings::qdev_init_clock_in(
                    dev,
                    cstr.as_ptr(),
                    cb,
                    dev.cast::<c_void>(),
                    events.0,
                );

                Owned::from(&*clk)
            }
        }

        let cb: Option<unsafe extern "C" fn(*mut c_void, ClockEvent)> = if F::is_some() {
            unsafe extern "C" fn rust_clock_cb<T, F: for<'a> FnCall<(&'a T, ClockEvent)>>(
                opaque: *mut c_void,
                event: ClockEvent,
            ) {
                // SAFETY: the opaque is "this", which is indeed a pointer to T
                F::call((unsafe { &*(opaque.cast::<T>()) }, event))
            }
            Some(rust_clock_cb::<Self::Target, F>)
        } else {
            None
        };

        do_init_clock_in(self.as_mut_ptr(), name, cb, events)
    }

    /// Add an output clock named `name`.
    ///
    /// The resulting clock is added as a child of `self`, but it also
    /// stays alive until after `Drop::drop` is called because C code
    /// keeps an extra reference to it until `device_finalize()` calls
    /// `qdev_finalize_clocklist()`.  Therefore (unlike most cases in
    /// which Rust code has a reference to a child object) it would be
    /// possible for this function to return a `&Clock` too.
    #[inline]
    fn init_clock_out(&self, name: &str) -> Owned<Clock> {
        unsafe {
            let cstr = CString::new(name).unwrap();
            let clk = bindings::qdev_init_clock_out(self.as_mut_ptr(), cstr.as_ptr());

            Owned::from(&*clk)
        }
    }

    fn prop_set_chr(&self, propname: &str, chr: &Owned<Chardev>) {
        assert!(bql_locked());
        let c_propname = CString::new(propname).unwrap();
        unsafe {
            bindings::qdev_prop_set_chr(self.as_mut_ptr(), c_propname.as_ptr(), chr.as_mut_ptr());
        }
    }
}

impl<R: ObjectDeref> DeviceMethods for R where R::Target: IsA<DeviceState> {}

unsafe impl ObjectType for Clock {
    type Class = ObjectClass;
    const TYPE_NAME: &'static CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(bindings::TYPE_CLOCK) };
}
qom_isa!(Clock: Object);
