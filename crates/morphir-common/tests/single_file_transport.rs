use std::cell::Cell;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::rc::Rc;

use morphir_common::ir_transport::{ClassicV3ModuleVisitor, visit_classic_v3};
use morphir_core::ir::classic;

struct ChunkedReader<R> {
    inner: R,
    largest_read: Rc<Cell<usize>>,
    bytes_read: Rc<Cell<usize>>,
}

impl<R: Read> Read for ChunkedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(1024);
        self.largest_read.set(self.largest_read.get().max(length));
        let read = self.inner.read(&mut buffer[..length])?;
        self.bytes_read.set(self.bytes_read.get() + read);
        Ok(read)
    }
}

impl<R: Seek> Seek for ChunkedReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[derive(Default)]
struct ModuleCounter {
    began: bool,
    modules: usize,
    bytes_read: Rc<Cell<usize>>,
    first_module_offset: Option<usize>,
}

struct CallbackObserver {
    began: Rc<Cell<bool>>,
    modules: Rc<Cell<usize>>,
}

impl ClassicV3ModuleVisitor for CallbackObserver {
    type Output = ();

    fn begin(
        &mut self,
        _package: &classic::Path,
        _dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> Result<(), String> {
        self.began.set(true);
        Ok(())
    }

    fn visit_module(
        &mut self,
        _module: classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), String> {
        self.modules.set(self.modules.get() + 1);
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, String> {
        Ok(())
    }
}

impl ClassicV3ModuleVisitor for ModuleCounter {
    type Output = (usize, usize);

    fn begin(
        &mut self,
        _package: &classic::Path,
        _dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> Result<(), String> {
        self.began = true;
        Ok(())
    }

    fn visit_module(
        &mut self,
        _module: classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), String> {
        assert!(self.began);
        self.first_module_offset
            .get_or_insert_with(|| self.bytes_read.get());
        self.modules += 1;
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, String> {
        Ok((self.modules, self.first_module_offset.unwrap_or_default()))
    }
}

#[test]
fn classic_v3_source_visits_one_module_at_a_time_from_a_reader() {
    let bytes =
        include_bytes!("../../../../../website/static/ir/examples/v3/greeting-example.json");
    let largest_read = Rc::new(Cell::new(0));
    let bytes_read = Rc::new(Cell::new(0));
    let reader = ChunkedReader {
        inner: Cursor::new(bytes.as_slice()),
        largest_read: largest_read.clone(),
        bytes_read: bytes_read.clone(),
    };

    let (modules, first_module_offset) = visit_classic_v3(
        reader,
        ModuleCounter {
            bytes_read,
            ..ModuleCounter::default()
        },
    )
    .unwrap();

    assert!(modules > 1);
    assert!(first_module_offset < bytes.len());
    assert!(largest_read.get() <= 1024);
}

#[test]
fn large_lcr_source_releases_each_module_before_reading_the_rest() {
    let bytes = include_bytes!("../../../../../website/static/ir/examples/v3/lcr-morphir-ir.json");
    let largest_read = Rc::new(Cell::new(0));
    let bytes_read = Rc::new(Cell::new(0));
    let reader = ChunkedReader {
        inner: Cursor::new(bytes.as_slice()),
        largest_read: largest_read.clone(),
        bytes_read: bytes_read.clone(),
    };

    let (modules, first_module_offset) = visit_classic_v3(
        reader,
        ModuleCounter {
            bytes_read,
            ..ModuleCounter::default()
        },
    )
    .unwrap();

    assert_eq!(modules, 84);
    assert!(first_module_offset < bytes.len() / 10);
    assert!(largest_read.get() <= 1024);
}

#[test]
fn rejects_non_v3_input_before_invoking_visitor_callbacks() {
    let mut source: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../website/static/ir/examples/v3/greeting-example.json"
    ))
    .unwrap();
    source["formatVersion"] = 2.into();
    let source = serde_json::to_vec(&source).unwrap();
    let began = Rc::new(Cell::new(false));
    let modules = Rc::new(Cell::new(0));

    let result = visit_classic_v3(
        Cursor::new(source.as_slice()),
        CallbackObserver {
            began: began.clone(),
            modules: modules.clone(),
        },
    );

    assert!(result.is_err());
    assert!(!began.get());
    assert_eq!(modules.get(), 0);
}
