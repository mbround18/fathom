use super::traits::BoxedView;
use std::collections::HashMap;

/// Registry that maps section keys to view component constructors
///
/// This enables declaring which view handles which section without
/// scattering if-statements throughout the codebase.
pub struct ViewRegistry {
    views: HashMap<String, Box<dyn Fn() -> BoxedView>>,
}

impl ViewRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            views: HashMap::new(),
        }
    }

    /// Register a view component factory for a section key
    pub fn register<F>(&mut self, section_key: &str, factory: F)
    where
        F: Fn() -> BoxedView + 'static,
    {
        self.views
            .insert(section_key.to_string(), Box::new(factory));
    }

    /// Get or create a view for the given section key
    ///
    /// Returns None if the section is not registered.
    pub fn get_or_create(&self, section_key: &str) -> Option<BoxedView> {
        self.views.get(section_key).map(|factory| factory())
    }

    /// Check if a section has a registered view
    pub fn has_view(&self, section_key: &str) -> bool {
        self.views.contains_key(section_key)
    }
}

impl Default for ViewRegistry {
    fn default() -> Self {
        Self::new()
    }
}
