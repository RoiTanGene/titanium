
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

/*
 * TODO: switch from AtomicIsize to AtomicU64.
 */

extern crate dbus;
#[macro_use]
extern crate dbus_macros;
#[macro_use]
extern crate webkit2gtk_webextension;

mod dom;
mod scroll;
mod message_server;

use std::sync::Arc;
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::Ordering::Relaxed;
use std::thread;

use glib::variant::Variant;
use webkit2gtk_webextension::WebExtension;

use message_server::MessageServer;

web_extension_init!();

#[no_mangle]
pub fn web_extension_initialize(extension: WebExtension, user_data: Variant) {
    let current_page_id = Arc::new(AtomicIsize::new(-1));

    {
        let current_page_id = current_page_id.clone();
        extension.connect_page_created(move |_, page| {
            current_page_id.store(page.get_id() as isize, Relaxed);
        });
    }

    let bus_name = user_data.get_str();
    if let Some(bus_name) = bus_name {
        let bus_name = bus_name.to_string();
        let message_server = MessageServer::new(current_page_id, extension);
        thread::spawn(move || message_server.run(&bus_name));
    }
}
