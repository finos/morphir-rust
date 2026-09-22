//! Test-only interception of the current native SQLite file's methods.
//!
//! No replacement filesystem is registered. All other methods and successful calls
//! use the original native VFS. The exclusive connection borrow pins the file's
//! lifetime; xClose removes intercepted entries before native deallocation.
use rusqlite::{Connection, ffi};
use std::{cell::RefCell, collections::BTreeMap, ffi::c_void, marker::PhantomData, ptr, rc::Rc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Write,
    Sync,
}
#[derive(Clone, Copy)]
pub enum File {
    Database,
    Wal,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub writes: usize,
    pub syncs: usize,
    pub failures: usize,
}
struct Entry {
    original: *const ffi::sqlite3_io_methods,
    _replacement: Box<ffi::sqlite3_io_methods>,
    failure: Option<(Operation, usize)>,
    counts: Rc<RefCell<Counts>>,
}
thread_local! { static FILES: RefCell<BTreeMap<usize, Entry>> = const { RefCell::new(BTreeMap::new()) }; }

pub struct Intercept<'a> {
    connection: &'a mut Connection,
    file: *mut ffi::sqlite3_file,
    counts: Rc<RefCell<Counts>>,
    // Native callbacks and the connection borrow stay on this thread.
    _thread: PhantomData<Rc<()>>,
}
impl<'a> Intercept<'a> {
    pub fn install(
        connection: &'a mut Connection,
        target: File,
        failure: Option<(Operation, usize)>,
    ) -> Self {
        let mut file = ptr::null_mut::<ffi::sqlite3_file>();
        let opcode = match target {
            File::Database => ffi::SQLITE_FCNTL_FILE_POINTER,
            File::Wal => ffi::SQLITE_FCNTL_JOURNAL_POINTER,
        };
        // SAFETY: exclusive live connection and correctly typed writable file-pointer output.
        assert_eq!(
            unsafe {
                ffi::sqlite3_file_control(
                    connection.handle(),
                    ptr::null(),
                    opcode,
                    (&mut file as *mut *mut ffi::sqlite3_file).cast(),
                )
            },
            ffi::SQLITE_OK
        );
        assert!(!file.is_null(), "requested native file is not open");
        // SAFETY: SQLite returned a live native file. Its methods remain valid until xClose.
        let original = unsafe { (*file).pMethods };
        assert!(!original.is_null());
        // SAFETY: version 1 fields always exist; initialize later optional fields only
        // when the native table advertises that layout.
        let mut replacement: Box<ffi::sqlite3_io_methods> = Box::new(unsafe { std::mem::zeroed() });
        unsafe {
            let version = (*original).iVersion;
            let bytes = match version {
                1 => std::mem::offset_of!(ffi::sqlite3_io_methods, xShmMap),
                2 => std::mem::offset_of!(ffi::sqlite3_io_methods, xFetch),
                3 => std::mem::size_of::<ffi::sqlite3_io_methods>(),
                _ => panic!("unsupported native file method version {version}"),
            };
            ptr::copy_nonoverlapping(
                original.cast::<u8>(),
                (&mut *replacement as *mut ffi::sqlite3_io_methods).cast(),
                bytes,
            );
        }
        assert!(
            replacement.xWrite.is_some()
                && replacement.xSync.is_some()
                && replacement.xClose.is_some()
        );
        replacement.xWrite = Some(write);
        replacement.xSync = Some(sync);
        replacement.xClose = Some(close);
        let methods = &*replacement as *const _;
        let counts = Rc::new(RefCell::new(Counts::default()));
        FILES.with(|files| {
            let mut files = files.borrow_mut();
            let std::collections::btree_map::Entry::Vacant(slot) = files.entry(file as usize)
            else {
                panic!("native file already intercepted");
            };
            slot.insert(Entry {
                original,
                _replacement: replacement,
                failure,
                counts: counts.clone(),
            });
        });
        // SAFETY: replacement is pinned in a Box until restoration or xClose.
        unsafe {
            (*file).pMethods = methods;
        }
        Self {
            connection,
            file,
            counts,
            _thread: PhantomData,
        }
    }
    pub fn connection(&mut self) -> &mut Connection {
        self.connection
    }
    pub fn counts(&self) -> Counts {
        *self.counts.borrow()
    }
}
impl Drop for Intercept<'_> {
    fn drop(&mut self) {
        restore(self.file);
    }
}
fn restore(file: *mut ffi::sqlite3_file) -> Option<*const ffi::sqlite3_io_methods> {
    FILES
        .with(|files| files.borrow_mut().remove(&(file as usize)))
        .map(|entry| {
            // SAFETY: entries are removed by xClose before native deallocation, so a
            // present entry still names a live file. No concurrent connection use.
            unsafe {
                (*file).pMethods = entry.original;
            }
            entry.original
        })
}
fn operation(
    file: *mut ffi::sqlite3_file,
    operation: Operation,
) -> Option<(*const ffi::sqlite3_io_methods, bool)> {
    FILES.with(|files| {
        files.borrow_mut().get_mut(&(file as usize)).map(|entry| {
            let mut counts = entry.counts.borrow_mut();
            match operation {
                Operation::Write => counts.writes += 1,
                Operation::Sync => counts.syncs += 1,
            }
            let count = match operation {
                Operation::Write => counts.writes,
                Operation::Sync => counts.syncs,
            };
            let fail = entry
                .failure
                .is_some_and(|(selected, at)| selected == operation && count >= at);
            if fail {
                counts.failures += 1;
            }
            (entry.original, fail)
        })
    })
}
unsafe extern "C" fn write(
    file: *mut ffi::sqlite3_file,
    buffer: *const c_void,
    amount: i32,
    offset: i64,
) -> i32 {
    match operation(file, Operation::Write) {
        Some((_, true)) => ffi::SQLITE_IOERR_WRITE,
        // SAFETY: unmodified native file and arguments received from SQLite.
        Some((methods, false)) => unsafe {
            ((*methods).xWrite.unwrap())(file, buffer, amount, offset)
        },
        None => ffi::SQLITE_IOERR_WRITE,
    }
}
unsafe extern "C" fn sync(file: *mut ffi::sqlite3_file, flags: i32) -> i32 {
    match operation(file, Operation::Sync) {
        Some((_, true)) => ffi::SQLITE_IOERR_FSYNC,
        // SAFETY: unmodified native file and arguments received from SQLite.
        Some((methods, false)) => unsafe { ((*methods).xSync.unwrap())(file, flags) },
        None => ffi::SQLITE_IOERR_FSYNC,
    }
}
unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> i32 {
    match restore(file) {
        // SAFETY: restore reinstated the original method table before native close.
        Some(methods) => unsafe { ((*methods).xClose.unwrap())(file) },
        None => ffi::SQLITE_IOERR_CLOSE,
    }
}
