/*
 * Copyright (c) 2016 Boucher, Antoni <bouanto@zoho.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal in
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
 * the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
 * FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
 * COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
 * IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */

use webkit2gtk_webextension::{DOMElementExt, WebPage};

use dom::{get_body, get_document};

/// Trait for widget that can scroll.
pub trait Scrollable {
    fn scroll_by(&self, pixels: i64);
    fn scroll_bottom(&self);
    fn scroll_percentage(&self) -> i64;
    fn scroll_top(&self);
}

impl Scrollable for WebPage {
    /// Scroll the web page vertically by the specified amount of pixels.
    /// A negative value scroll towards to top.
    fn scroll_by(&self, pixels: i64) {
        if let Some(body) = get_body(self) {
            body.set_scroll_top(body.get_scroll_top() + pixels);
        }
    }

    /// Scroll to the bottom of the web page.
    fn scroll_bottom(&self) {
        if let Some(body) = get_body(self) {
            body.set_scroll_top(body.get_scroll_height());
        }
    }

    /// Get the current vertical scroll position of the web page as a percentage.
    fn scroll_percentage(&self) -> i64 {
        let default = -1;
        if let (Some(body), Some(document)) = (get_body(self), get_document(self)) {
            let height = document.get_client_height();
            let scroll_height = body.get_scroll_height();
            if scroll_height <= height as i64 {
                default
            }
            else {
                (body.get_scroll_top() as f64 / (scroll_height as f64 - height) * 100.0) as i64
            }
        }
        else {
            default
        }
    }

    /// Scroll to the top of the web page.
    fn scroll_top(&self) {
        if let Some(body) = get_body(self) {
            body.set_scroll_top(0);
        }
    }
}
