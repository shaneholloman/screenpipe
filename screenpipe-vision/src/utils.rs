use crate::capture_screenshot_by_window::{
    capture_all_visible_windows, CapturedWindow, WindowFilters,
};
use crate::core::MaxAverageFrame;
use crate::custom_ocr::CustomOcrConfig;
use crate::monitor::SafeMonitor;
use image::DynamicImage;
use image_compare::{Algorithm, Metric, Similarity};
use tracing::{debug, warn};
use screenpipe_db::CustomOcrConfig as DBCustomOcrConfig;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub enum OcrEngine {
    Unstructured,
    #[default]
    Tesseract,
    WindowsNative,
    AppleNative,
    Custom(CustomOcrConfig),
}

impl From<OcrEngine> for screenpipe_db::OcrEngine {
    fn from(val: OcrEngine) -> Self {
        match val {
            OcrEngine::Unstructured => screenpipe_db::OcrEngine::Unstructured,
            OcrEngine::Tesseract => screenpipe_db::OcrEngine::Tesseract,
            OcrEngine::WindowsNative => screenpipe_db::OcrEngine::WindowsNative,
            OcrEngine::AppleNative => screenpipe_db::OcrEngine::AppleNative,
            OcrEngine::Custom(config) => {
                screenpipe_db::OcrEngine::Custom(DBCustomOcrConfig::from(config))
            }
        }
    }
}

impl From<screenpipe_db::OcrEngine> for OcrEngine {
    fn from(engine: screenpipe_db::OcrEngine) -> Self {
        match engine {
            screenpipe_db::OcrEngine::Unstructured => OcrEngine::Unstructured,
            screenpipe_db::OcrEngine::Tesseract => OcrEngine::Tesseract,
            screenpipe_db::OcrEngine::WindowsNative => OcrEngine::WindowsNative,
            screenpipe_db::OcrEngine::AppleNative => OcrEngine::AppleNative,
            screenpipe_db::OcrEngine::Custom(config) => OcrEngine::Custom(config.into()),
        }
    }
}

pub fn calculate_hash(image: &DynamicImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.as_bytes().hash(&mut hasher);
    hasher.finish()
}

pub fn compare_images_histogram(
    image1: &DynamicImage,
    image2: &DynamicImage,
) -> anyhow::Result<f64> {
    let image_one = image1.to_luma8();
    let image_two = image2.to_luma8();
    image_compare::gray_similarity_histogram(Metric::Hellinger, &image_one, &image_two)
        .map_err(|e| anyhow::anyhow!("Failed to compare images: {}", e))
}

pub fn compare_images_ssim(image1: &DynamicImage, image2: &DynamicImage) -> f64 {
    let image_one = image1.to_luma8();
    let image_two = image2.to_luma8();
    let result: Similarity =
        image_compare::gray_similarity_structure(&Algorithm::MSSIMSimple, &image_one, &image_two)
            .expect("Images had different dimensions");
    result.score
}

pub async fn capture_screenshot(
    monitor: &SafeMonitor,
    window_filters: &WindowFilters,
    capture_unfocused_windows: bool,
) -> Result<(DynamicImage, Vec<CapturedWindow>, u64, Duration), anyhow::Error> {
    // info!("Starting screenshot capture for monitor: {:?}", monitor);
    let capture_start = Instant::now();
    let image = monitor.capture_image().await.map_err(|e| {
        debug!("failed to capture monitor image: {}", e);
        anyhow::anyhow!("monitor capture failed")
    })?;
    let image_hash = calculate_hash(&image);
    let capture_duration = capture_start.elapsed();

    let window_images =
        match capture_all_visible_windows(monitor, window_filters, capture_unfocused_windows).await
        {
            Ok(images) => images,
            Err(e) => {
                warn!(
                    "Failed to capture window images: {}. Continuing with empty result.",
                    e
                );
                Vec::new()
            }
        };

    Ok((image, window_images, image_hash, capture_duration))
}

pub async fn compare_with_previous_image(
    previous_image: Option<&DynamicImage>,
    current_image: &DynamicImage,
    max_average: &mut Option<MaxAverageFrame>,
    frame_number: u64,
    max_avg_value: &mut f64,
) -> anyhow::Result<f64> {
    let mut current_average = 0.0;
    if let Some(prev_image) = previous_image {
        let histogram_diff = compare_images_histogram(prev_image, current_image)?;
        let ssim_diff = 1.0 - compare_images_ssim(prev_image, current_image);
        current_average = (histogram_diff + ssim_diff) / 2.0;
        let max_avg_frame_number = max_average.as_ref().map_or(0, |frame| frame.frame_number);
        debug!(
            "Frame {}: Histogram diff: {:.3}, SSIM diff: {:.3}, Current Average: {:.3}, Max_avr: {:.3} Fr: {}",
            frame_number, histogram_diff, ssim_diff, current_average, *max_avg_value, max_avg_frame_number
        );
    } else {
        debug!("No previous image to compare for frame {}", frame_number);
    }
    Ok(current_average)
}
