use regex::{Regex, RegexBuilder};

use crate::ue;

#[derive(Debug, Clone)]
pub struct ObjectCache {
    pub name: String,
    pub script_status: Option<(usize, Result<String, String>)>,
    pub class_name: String,
    pub is_class_default_object: bool,
    pub parent_classes: Vec<String>,
}

impl ObjectCache {
    pub fn new(object: &ue::UObjectBase) -> Self {
        let script_status = if let Some(func) = object.cast::<ue::UFunction>() {
            let mut stream = std::io::Cursor::new(func.script.as_slice());
            let ex = crate::kismet::read_all(&mut stream);
            Some((
                func.script.len(),
                ex.map(|ex| format!("{}", ex.len()))
                    .map_err(|e| e.to_string()),
            ))
        } else {
            None
        };

        let class = object.class();
        let class_name = class.path().to_string();
        let is_class_default_object = false; //object.is_class_default_object();

        let mut parent_classes = Vec::new();
        let mut current_class = unsafe { class.super_struct.as_ref() };
        while let Some(parent) = current_class {
            parent_classes.push(parent.path().to_string());
            current_class = unsafe { parent.super_struct.as_ref() };
        }

        Self {
            name: object.path(),
            script_status,
            class_name,
            is_class_default_object,
            parent_classes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchFlags {
    pub include_class_default_objects: bool,
    pub include_instances: bool,
    pub search_parent_classes: bool,
    pub class_name_filter: Option<String>,
}

impl Default for SearchFlags {
    fn default() -> Self {
        Self {
            include_class_default_objects: true,
            include_instances: true,
            search_parent_classes: false,
            class_name_filter: None,
        }
    }
}

pub struct ObjectFilter {
    name_search: String,
    re: Option<Regex>,
    pub flags: SearchFlags,
}

impl ObjectFilter {
    pub fn new(search: String) -> Self {
        let mut new = Self {
            name_search: String::new(),
            re: None,
            flags: SearchFlags::default(),
        };
        new.set_search(search);
        new
    }

    pub fn get_search(&self) -> &str {
        &self.name_search
    }

    pub fn set_search(&mut self, value: String) {
        self.name_search = value;
        self.re = RegexBuilder::new(&self.name_search)
            .case_insensitive(true)
            .build()
            .ok()
    }

    pub fn matches(&self, object: &ObjectCache) -> bool {
        if !self.flags.include_class_default_objects && object.is_class_default_object {
            return false;
        }

        if !self.flags.include_instances && !object.is_class_default_object {
            return false;
        }

        if let Some(class_filter) = &self.flags.class_name_filter {
            let matches_class = object
                .class_name
                .to_lowercase()
                .contains(&class_filter.to_lowercase());
            let matches_parent = self.flags.search_parent_classes
                && object
                    .parent_classes
                    .iter()
                    .any(|p| p.to_lowercase().contains(&class_filter.to_lowercase()));

            if !matches_class && !matches_parent {
                return false;
            }
        }

        if let Some(re) = &self.re {
            let matches_name = re.is_match(&object.name);
            let matches_class = re.is_match(&object.class_name);
            let matches_parent = self.flags.search_parent_classes
                && object.parent_classes.iter().any(|p| re.is_match(p));

            matches_name || matches_class || matches_parent
        } else {
            true
        }
    }
}
