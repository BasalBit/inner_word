//! Headless tests of Vim mode against a real `Editor` entity: keys go
//! through GPUI's dispatch path (interceptors, bindings, the platform input
//! handler), exactly as in the app.

mod suite;
