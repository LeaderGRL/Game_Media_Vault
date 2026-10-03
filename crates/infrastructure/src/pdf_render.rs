use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
};

use game_media_vault_application::{MediaTransformPort, PortError};
use game_media_vault_domain::DerivationRecipe;
use image::ImageFormat;
use pdfium_render::prelude::{PdfRenderConfig, Pdfium};

use crate::image_transform::{DEFAULT_MAX_ORIGINAL_BYTES, read_original};

/// The environment variable naming a directory that holds the pdfium library.
pub const PDFIUM_DIRECTORY_VARIABLE: &str = "GAME_MEDIA_VAULT_PDFIUM";

/// The pdfium library this machine provides, bound once for the whole process.
static MACHINE_PDFIUM: OnceLock<Option<Arc<Pdfium>>> = OnceLock::new();

/// Serializes every call into pdfium, whose C library is not thread-safe: binding it, each
/// load, render and release of a document, and destroying it.
static PDFIUM_CALLS: Mutex<()> = Mutex::new(());

fn pdfium_calls() -> MutexGuard<'static, ()> {
    PDFIUM_CALLS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Renders PDF originals through the pdfium library: a thumbnail is the first page drawn to fit
/// the longest edge. The library is loaded only from explicit directories, never through the
/// system search path, so a library planted in the working directory is never loaded.
pub struct PdfiumRenderer {
    pdfium: Option<Arc<Pdfium>>,
    max_original_bytes: u64,
}

impl PdfiumRenderer {
    /// The pdfium library of `directory`, or none when it holds no library to bind.
    pub fn from_directory(directory: &Path) -> Self {
        Self::with(bind(directory))
    }

    /// The pdfium library of this machine: in the directory `GAME_MEDIA_VAULT_PDFIUM` names,
    /// otherwise beside the running executable.
    pub fn machine() -> Self {
        let pdfium = MACHINE_PDFIUM.get_or_init(|| {
            let configured = std::env::var_os(PDFIUM_DIRECTORY_VARIABLE).map(PathBuf::from);
            let beside_executable = std::env::current_exe()
                .ok()
                .and_then(|executable| executable.parent().map(Path::to_path_buf));
            configured
                .into_iter()
                .chain(beside_executable)
                .find_map(|directory| bind(&directory))
        });
        Self::with(pdfium.clone())
    }

    fn with(pdfium: Option<Arc<Pdfium>>) -> Self {
        Self {
            pdfium,
            max_original_bytes: DEFAULT_MAX_ORIGINAL_BYTES,
        }
    }

    /// Whether a pdfium library was found, without which PDF originals are not rendered.
    pub fn is_available(&self) -> bool {
        self.pdfium.is_some()
    }
}

fn bind(directory: &Path) -> Option<Arc<Pdfium>> {
    let _serialized = pdfium_calls();
    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(directory))
        .ok()
        .map(|bindings| Arc::new(Pdfium::new(bindings)))
}

impl Drop for PdfiumRenderer {
    fn drop(&mut self) {
        // Destroying the library, when this renderer holds its last reference, calls pdfium too.
        let _serialized = pdfium_calls();
        self.pdfium.take();
    }
}

impl MediaTransformPort for PdfiumRenderer {
    fn can_transform(&self, media_type: &str, recipe: &DerivationRecipe) -> bool {
        self.pdfium.is_some()
            && media_type == "application/pdf"
            && matches!(recipe, DerivationRecipe::Thumbnail { .. })
    }

    fn transform(
        &self,
        original: &mut dyn Read,
        _media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        let (Some(pdfium), DerivationRecipe::Thumbnail { max_edge }) = (&self.pdfium, recipe)
        else {
            return Err(PortError::new(
                "pdfium renders the thumbnails of PDF originals once this machine provides it"
                    .to_owned(),
            ));
        };
        let bytes = read_original(original, self.max_original_bytes)?;
        // Held until the page and the document are released, which are pdfium calls too.
        let _serialized = pdfium_calls();
        let document = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(|error| PortError::new(format!("cannot open the PDF: {error}")))?;
        let page = document
            .pages()
            .get(0)
            .map_err(|error| PortError::new(format!("cannot read the first page: {error}")))?;
        // The page keeps its proportions, its longest edge drawn at the recipe's.
        let (width, height) = (page.width().value, page.height().value);
        let scale = *max_edge as f32 / width.max(height);
        let config = PdfRenderConfig::new().set_target_size(
            ((width * scale).round() as i32).max(1),
            ((height * scale).round() as i32).max(1),
        );
        let image = page
            .render_with_config(&config)
            .map_err(|error| PortError::new(format!("cannot render the first page: {error}")))?
            .as_image()
            .map_err(|error| PortError::new(format!("cannot render the first page: {error}")))?;
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .map_err(|error| PortError::new(format!("failed to encode the output: {error}")))?;
        Ok(encoded)
    }
}

/// Several transformers, each media type going to the first that can apply the recipe to it.
pub struct MediaTransformers(Vec<Box<dyn MediaTransformPort>>);

impl MediaTransformers {
    pub fn new(transformers: Vec<Box<dyn MediaTransformPort>>) -> Self {
        Self(transformers)
    }

    /// The transformers of this machine: raster images, and PDF originals once it provides
    /// pdfium.
    pub fn machine() -> Self {
        Self::new(vec![
            Box::new(crate::ImageTransformer::new()),
            Box::new(PdfiumRenderer::machine()),
        ])
    }

    fn first_for(
        &self,
        media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Option<&dyn MediaTransformPort> {
        self.0
            .iter()
            .map(Box::as_ref)
            .find(|transformer| transformer.can_transform(media_type, recipe))
    }
}

impl MediaTransformPort for MediaTransformers {
    fn can_transform(&self, media_type: &str, recipe: &DerivationRecipe) -> bool {
        self.first_for(media_type, recipe).is_some()
    }

    fn transform(
        &self,
        original: &mut dyn Read,
        media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        self.first_for(media_type, recipe)
            .ok_or_else(|| {
                PortError::new(format!(
                    "no transformer applies this recipe to {media_type}"
                ))
            })?
            .transform(original, media_type, recipe)
    }
}
