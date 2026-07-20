//! Human behavior simulation: mouse movements, scrolling, typing, random delays.
//!
//! Uses CDP Input domain for realistic interactions that bypass behavioral detection.

use rand::Rng;
use serde_json::json;
use std::time::Duration;

use crate::error::Result;
use crate::page::Page;

/// Human-like behavior simulator bound to a page.
pub struct HumanBehavior<'a> {
    page: &'a Page,
}

impl<'a> HumanBehavior<'a> {
    pub fn new(page: &'a Page) -> Self {
        Self { page }
    }

    /// Random delay with gaussian-like distribution between min and max.
    pub async fn random_delay(&self, min_ms: u64, max_ms: u64) -> Result<()> {
        let delay = rand::rng().random_range(min_ms..=max_ms);
        tokio::time::sleep(Duration::from_millis(delay)).await;
        Ok(())
    }

    /// Get element center coordinates via JS.
    async fn get_element_center(&self, selector: &str) -> Result<(f64, f64)> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({});
                if (!el) return null;
                const r = el.getBoundingClientRect();
                return {{ x: r.x + r.width / 2, y: r.y + r.height / 2 }};
            }})()"#,
            serde_json::to_string(selector).unwrap()
        );
        let result = self.page.evaluate(&js).await?;
        let x = result.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = result.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        Ok((x, y))
    }

    /// Move mouse to element along a bezier curve path.
    pub async fn move_mouse_to(&self, selector: &str) -> Result<()> {
        let (target_x, target_y) = self.get_element_center(selector).await?;

        // Start from a random position
        let mut rng = rand::rng();
        let start_x = rng.random_range(0.0..400.0);
        let start_y = rng.random_range(0.0..300.0);

        // Generate bezier curve control points
        let cp1_x = start_x + (target_x - start_x) * 0.3 + rng.random_range(-50.0..50.0);
        let cp1_y = start_y + (target_y - start_y) * 0.3 + rng.random_range(-50.0..50.0);
        let cp2_x = start_x + (target_x - start_x) * 0.7 + rng.random_range(-30.0..30.0);
        let cp2_y = start_y + (target_y - start_y) * 0.7 + rng.random_range(-30.0..30.0);

        // Interpolate along the curve (10-20 steps)
        let steps = rng.random_range(10..=20);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let x = cubic_bezier(start_x, cp1_x, cp2_x, target_x, t);
            let y = cubic_bezier(start_y, cp1_y, cp2_y, target_y, t);

            self.page.cmd("Input.dispatchMouseEvent", json!({
                "type": "mouseMoved",
                "x": x,
                "y": y,
            })).await?;

            // Small delay between movements (5-25ms)
            tokio::time::sleep(Duration::from_millis(rng.random_range(5..=25))).await;
        }

        Ok(())
    }

    /// Human-like click: move to element + short pause + press + release.
    pub async fn human_click(&self, selector: &str) -> Result<()> {
        self.move_mouse_to(selector).await?;
        let (x, y) = self.get_element_center(selector).await?;

        let mut rng = rand::rng();
        // Small offset to avoid clicking exact center every time
        let x = x + rng.random_range(-2.0..2.0);
        let y = y + rng.random_range(-2.0..2.0);

        // Pause before clicking (50-150ms)
        tokio::time::sleep(Duration::from_millis(rng.random_range(50..=150))).await;

        self.page.cmd("Input.dispatchMouseEvent", json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1,
        })).await?;

        // Hold for 30-80ms
        tokio::time::sleep(Duration::from_millis(rng.random_range(30..=80))).await;

        self.page.cmd("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1,
        })).await?;

        Ok(())
    }

    /// Human-like typing: character by character with random intervals.
    pub async fn human_type(&self, selector: &str, text: &str) -> Result<()> {
        // Focus the element first
        self.human_click(selector).await?;
        self.random_delay(200, 500).await?;

        let mut rng = rand::rng();
        for ch in text.chars() {
            self.page.cmd("Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "text": ch.to_string(),
            })).await?;
            self.page.cmd("Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "text": ch.to_string(),
            })).await?;

            // Random typing speed (40-180ms per char, occasional longer pause)
            let delay = if rng.random_range(0..10) < 2 {
                rng.random_range(150..=400) // occasional pause (thinking)
            } else {
                rng.random_range(40..=180)
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        Ok(())
    }

    /// Scroll the page by a given number of pixels (smooth scroll).
    pub async fn scroll(&self, pixels: i32) -> Result<()> {
        let steps = (pixels.abs() / 50).max(3);
        let step_size = pixels as f64 / steps as f64;

        for _ in 0..steps {
            self.page.evaluate(&format!("window.scrollBy(0, {})", step_size)).await?;
            tokio::time::sleep(Duration::from_millis(rand::rng().random_range(10..=40))).await;
        }
        Ok(())
    }

    /// Random scroll (up or down, random amount).
    pub async fn random_scroll(&self) -> Result<()> {
        let pixels: i32 = rand::rng().random_range(100..600);
        let direction: i32 = if rand::rng().random_range(0..10) < 7 { 1 } else { -1 };
        self.scroll(pixels * direction).await
    }

    /// Simulate browsing behavior: random scrolls + pauses over a duration.
    pub async fn browse(&self, duration: Duration) -> Result<()> {
        let start = tokio::time::Instant::now();
        let mut rng = rand::rng();

        while start.elapsed() < duration {
            // Random action: scroll (70%), pause (20%), mouse move (10%)
            match rng.random_range(0..10) {
                0..=6 => {
                    self.random_scroll().await?;
                    self.random_delay(500, 2000).await?;
                }
                7..=8 => {
                    self.random_delay(1000, 3000).await?;
                }
                _ => {
                    // Move mouse to random position
                    let x: f64 = rng.random_range(100.0..800.0);
                    let y: f64 = rng.random_range(100.0..500.0);
                    self.page.cmd("Input.dispatchMouseEvent", json!({
                        "type": "mouseMoved",
                        "x": x,
                        "y": y,
                    })).await?;
                    self.random_delay(300, 1000).await?;
                }
            }
        }

        Ok(())
    }
}

/// Cubic bezier interpolation.
fn cubic_bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}
