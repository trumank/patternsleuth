#[cfg(feature = "image-elf")]
pub mod elf;
mod macros;
#[cfg(feature = "image-pe")]
pub mod pe;

use crate::*;
use anyhow::Error;
#[cfg(feature = "image-elf")]
use elf::ElfImage;
#[cfg(feature = "image-pe")]
use pe::PEImage;

use macros::*;

image_type_dispatch! {
    @enum ImageType as _image_type_reflection {
        PEImage(PEImage, "image-pe"),
        ElfImage(ElfImage, "image-elf"),
    }

    @fns {
        fn get_function(address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError>;
        fn get_root_function(address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError>;
        fn get_root_function_range(address: usize) -> Result<Option<Range<usize>>, MemoryAccessError>;
        fn get_child_functions(address: usize) -> Result<Vec<RuntimeFunction>, MemoryAccessError>;
        fn get_root_functions() -> Result<Vec<Range<usize>>, MemoryAccessError>;
    }
}

pub use _image_type_reflection as image_type_reflection;
use pe::PEImageBuilder;

pub struct Image<'data> {
    pub base_address: usize,
    pub memory: Box<dyn SectionedMemoryTrait<'data> + 'data>,
    #[cfg(feature = "symbols")]
    pub symbols: Option<HashMap<usize, symbols::Symbol>>,
    pub imports: HashMap<String, HashMap<String, usize>>,
    pub image_type: ImageType,
}

// Type-independent
impl<'data> Image<'data> {
    #[allow(unused)]
    pub fn read<P: AsRef<Path>>(
        base_addr: Option<usize>,
        data: &'data [u8],
        exe_path: Option<P>,
        cache_functions: bool,
    ) -> Result<Image<'data>> {
        let object = object::File::parse(data)?;
        match object {
            #[cfg(feature = "image-elf")]
            object::File::Elf64(_) => {
                ElfImage::read_inner(base_addr, exe_path, cache_functions, object)
            }
            #[cfg(feature = "image-pe")]
            object::File::Pe64(_) => PEImageBuilder::new()
                .memory_from_object(object)?
                .exe_path(exe_path.as_ref().map(AsRef::as_ref))
                .build(),
            _ => Err(Error::msg("Unsupported file format")),
        }
    }
    pub fn builder() -> ImageBuilder {
        Default::default()
    }

    pub fn scan<'patterns, S>(
        &self,
        pattern_configs: &'patterns [PatternConfig<S>],
    ) -> Result<ScanResult<'patterns, S>> {
        let mut results = vec![];

        struct PendingScan {
            original_config_index: usize,
            scan: Scan,
        }

        let scan_queue = pattern_configs
            .iter()
            .enumerate()
            .map(|(index, config)| PendingScan {
                original_config_index: index,
                scan: config.scan.clone(), // TODO clone isn't ideal but makes handling multi-stage scans a lot easier
            })
            .collect::<Vec<_>>();

        for section in self.memory.sections() {
            let base_address = section.address();
            let data = section.data();

            let (pattern_scans, patterns): (Vec<_>, Vec<_>) = scan_queue
                .iter()
                .filter_map(|scan| {
                    scan.scan
                        .scan_type
                        .get_pattern()
                        .map(|pattern| (scan, pattern))
                })
                .unzip();

            let (xref_scans, xrefs): (Vec<_>, Vec<_>) = scan_queue
                .iter()
                .filter_map(|scan| scan.scan.scan_type.get_xref().map(|xref| (scan, xref)))
                .unzip();

            let scan_results = scanner::scan_pattern(&patterns, base_address, data)
                .into_iter()
                .chain(scanner::scan_xref(&xrefs, base_address, data))
                .zip(pattern_scans.iter().chain(xref_scans.iter()));

            for (addresses, scan) in scan_results {
                for address in addresses {
                    results.push((
                        &pattern_configs[scan.original_config_index],
                        Resolution { address },
                    ));
                }
            }
        }

        Ok(ScanResult { results })
    }
}

#[derive(Default)]
pub struct ImageBuilder {
    functions: bool,
}
pub struct ImageBuilderWithSymbols<P: AsRef<Path>> {
    symbols: Option<P>,
    functions: bool,
}
impl ImageBuilder {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols<P: AsRef<Path>>(self, exe_path: P) -> ImageBuilderWithSymbols<P> {
        ImageBuilderWithSymbols {
            symbols: Some(exe_path),
            functions: self.functions,
        }
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read::<&str>(None, data, None, self.functions)
    }
}
impl<P: AsRef<Path>> ImageBuilderWithSymbols<P> {
    pub fn functions(mut self, functions: bool) -> Self {
        self.functions = functions;
        self
    }
    #[cfg(feature = "symbols")]
    pub fn symbols(mut self, exe_path: P) -> Self {
        self.symbols = Some(exe_path);
        self
    }
    pub fn build(self, data: &[u8]) -> Result<Image<'_>> {
        Image::read(None, data, self.symbols, self.functions)
    }
}
