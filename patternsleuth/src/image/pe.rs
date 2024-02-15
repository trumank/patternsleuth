use std::collections::HashMap;
use std::ops::Range;

use super::{Image, ImageType};
use crate::RuntimeFunction;

pub struct PEImage {
    pub exception_directory_range: Range<usize>,
    pub exception_children_cache: HashMap<usize, Vec<RuntimeFunction>>,
}

impl PEImage {
    pub fn get_function(&self, image: &super::Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, crate::MemoryAccessError> {
        // place holder only
        self.get_root_function(image, address)
    }
    pub fn get_root_function(&self, image: &super::Image<'_>, address: usize) -> Result<Option<RuntimeFunction>, crate::MemoryAccessError> {
        // place holder only
        let x = self.exception_directory_range.clone();
        if x.contains(&address) {
            Ok(Some(
                RuntimeFunction {
                    range: x,
                    unwind: 0
                }
            ))
        } else {
            Ok(None)
        }
    }
    pub fn get_child_functions(&self, image: &super::Image<'_>, address: usize) -> Result<Vec<RuntimeFunction>, crate::MemoryAccessError> {
        // place holder only
        match self.get_function(image, address) {
            Ok(Some(f)) => Ok(vec![f]),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e)
        }
    }
}

impl PEImage {
    pub fn read_inner<'data, P: AsRef<std::path::Path>>(
        exe_path: Option<P>,
        load_functions: bool,
        memory: crate::Memory<'_>,
        object: object::File<'_>,
    ) -> Result<Image<'data>, anyhow::Error> {
        let functions = if load_functions {
            // place holder only
            Some(vec![0..0x1000, 0x2000..0x3000])
        } else {
            None
        };
        Ok(Image {
            base_address: 0,
            memory,
            symbols: None,
            imports: Default::default(),
            image_type: ImageType::PEImage(PEImage {
                exception_directory_range: 0..0x1000,
                exception_children_cache: Default::default(),
            }),
        })
    }
}