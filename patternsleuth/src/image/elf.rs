use std::ops::Range;

use crate::{MemoryAccessError, RuntimeFunction};

use super::{Image, ImageType};

pub struct ElfImage {
    pub functions: Option<Vec<Range<usize>>>,
}

impl ElfImage {
    pub fn get_function(&self, image: &Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        self.get_root_function(image, address)
    }
    pub fn get_root_function(&self, image: &Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, MemoryAccessError> {
        let x = self.functions.as_ref().unwrap();
        x.iter().find(|p| p.contains(&address)).map(|a| 
            Ok(Some(
                RuntimeFunction {
                    range: a.clone(),
                    unwind: 0
                }
            ))
        ).unwrap_or(Ok(None))
    }
    pub fn get_child_functions(&self, image: &Image<'_>, address: usize) -> Result<Vec<RuntimeFunction>, MemoryAccessError> {
        match self.get_function(image, address) {
            Ok(Some(f)) => Ok(vec![f]),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e)
        }
    }
}

// read_inner 
impl ElfImage {
    pub fn read_inner<'data, P: AsRef<std::path::Path>>(
        exe_path: Option<P>,
        load_functions: bool,
        memory: crate::Memory<'_>,
        object: object::File<'_>,
    ) -> Result<Image<'data>, anyhow::Error> {
        let functions = if load_functions {
            Some(vec![0..0x1000, 0x2000..0x3000])
        } else {
            None
        };
        Ok(Image {
            base_address: 0,
            memory,
            symbols: None,
            imports: Default::default(),
            image_type: ImageType::ElfImage(ElfImage {
                functions,
            }),
        })
    }
}