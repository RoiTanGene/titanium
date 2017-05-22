/*
 * Copyright (c) 2017 Boucher, Antoni <bouanto@zoho.com>
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

use std::collections::HashMap;
use std::io;

use bincode;
use fg_uds::UnixStream;
use futures::{AsyncSink, Sink};
use futures_glib::MainContext;
use glib::Cast;
use relm_state::{Component, Relm, Update, execute};
use tokio_io::AsyncRead;
use tokio_io::codec::{FramedRead, FramedWrite};
use tokio_io::io::WriteHalf;
use webkit2gtk_webextension::{
    DOMDOMWindowExtManual,
    DOMDocumentExt,
    DOMElement,
    DOMElementExt,
    DOMHTMLElement,
    DOMHTMLElementExt,
    DOMHTMLInputElement,
    DOMHTMLSelectElement,
    DOMHTMLTextAreaElement,
    DOMNodeExt,
    WebExtension,
};

use titanium_common::{ExtCodec, Message};
use titanium_common::Action::{FileInput, GoInInsertMode, NoAction};
use titanium_common::Message::*;

use dom::{
    ElementIter,
    get_body,
    is_enabled,
    is_hidden,
    is_text_input,
    mouse_down,
    mouse_out,
    mouse_over,
};
use hints::{create_hints, hide_unrelevant_hints, show_all_hints, HINTS_ID};
use login_form::{get_credentials, load_password, load_username, submit_login_form};
use scroll::Scrollable;
use self::Msg::*;

macro_rules! get_document {
    ($_self:ident) => {{
        let document = get_page!($_self)
            .and_then(|page| page.get_dom_document());
        if let Some(document) = document {
            document
        }
        else {
            return;
        }
    }};
}

macro_rules! get_page {
    ($_self:ident) => {
        $_self.model.page_id
            .and_then(|page_id| $_self.model.extension.get_page(page_id))
    };
}

pub struct MessageClient {
    pub model: Model, // TODO: make it private.
}

pub struct Model {
    activated_file_input: Option<DOMHTMLInputElement>,
    pub extension: WebExtension, // TODO: make it private.
    hint_keys: String,
    hint_map: HashMap<String, DOMElement>,
    last_hovered_element: Option<DOMElement>,
    page_id: Option<u64>,
    writer: FramedWrite<WriteHalf<UnixStream>, ExtCodec>,
}

#[derive(Msg)]
pub enum Msg {
    MsgRecv(Message),
    MsgError(Box<bincode::ErrorKind>),
    SetPageId(u64),
}

impl Update for MessageClient {
    type Model = Model;
    type ModelParam = (UnixStream, WebExtension);
    type Msg = Msg;

    fn model(relm: &Relm<Self>, (stream, extension): Self::ModelParam) -> Model {
        let (reader, writer) = stream.split();
        let writer = FramedWrite::new(writer, ExtCodec);
        let reader = FramedRead::new(reader, ExtCodec);
        relm.connect_exec(reader, MsgRecv, MsgError);
        Model {
            activated_file_input: None,
            extension,
            hint_keys: String::new(),
            hint_map: HashMap::new(),
            last_hovered_element: None,
            page_id: None,
            writer,
        }
    }

    fn new(_relm: &Relm<Self>, model: Model) -> Option<Self> {
        Some(MessageClient {
            model,
        })
    }

    fn update(&mut self, event: Msg) {
        match event {
            MsgError(error) => println!("Error: {}", error),
            MsgRecv(message) => match message {
                ActivateHint(follow_mode) => self.activate_hint(&follow_mode),
                ActivateSelection() => self.activate_selection(),
                EnterHintKey(key) => self.enter_hint_key(key),
                FocusInput() => self.focus_input(),
                GetCredentials() => self.send_credentials(),
                GetScrollPercentage() => self.send_scroll_percentage(),
                HideHints() => self.hide_hints(),
                LoadPassword(password) => self.load_password(&password),
                LoadUsername(username) => self.load_username(&username),
                ScrollBottom() => self.scroll_bottom(),
                ScrollBy(pixels) => self.scroll_by(pixels),
                ScrollByX(pixels) => self.scroll_by_x(pixels),
                ScrollTop() => self.scroll_top(),
                SelectFile(file) => self.select_file(&file),
                ShowHints(hint_chars) => self.show_hints(&hint_chars),
                SubmitLoginForm() => self.submit_login_form(),
                _ => (), // TODO: log unexpected messages?
            },
            SetPageId(page_id) => self.model.page_id = Some(page_id),
        }
    }
}

impl MessageClient {
    pub fn new(path: &str, extension: WebExtension) -> io::Result<Component<Self>> {
        let cx = MainContext::default(|cx| cx.clone());
        let stream = UnixStream::connect(path, &cx)?;
        Ok(execute::<MessageClient>((stream, extension)))
    }

    // Activate (click, focus, hover) the selected hint.
    fn activate_hint(&mut self, follow_mode: &str) {
        let follow =
            if follow_mode == "hover" {
                MessageClient::hover
            }
            else {
                MessageClient::click
            };

        let element = self.model.hint_map.get(&self.model.hint_keys)
            .and_then(|element| element.clone().downcast::<DOMHTMLElement>().ok());
        match element {
            Some(element) => {
                self.hide_hints();
                self.model.hint_map.clear();
                self.model.hint_keys.clear();
                let action = follow(self, element);
                self.send(ActivateAction(action));
            },
            None => self.send(ActivateAction(NoAction as i32)),
        }
    }

    // Click on the link of the selected text.
    fn activate_selection(&self) -> () {
        let result = get_page!(self)
            .and_then(|page| page.get_dom_document())
            .and_then(|document| document.get_default_view())
            .and_then(|window| window.get_selection())
            .and_then(|selection| selection.get_anchor_node())
            .and_then(|anchor_node| anchor_node.get_parent_element())
            .and_then(|parent| parent.downcast::<DOMHTMLElement>().ok());
        if let Some(parent) = result {
            parent.click();
        }
    }

    fn click(&mut self, element: DOMHTMLElement) -> i32 {
        if element.is::<DOMHTMLInputElement>() {
            if let Ok(input_element) = element.clone().downcast::<DOMHTMLInputElement>() {
                let input_type = input_element.get_input_type();
                if let Some(input_type) = input_type {
                    match input_type.as_ref() {
                        "button" | "checkbox" | "image" | "radio" | "reset" | "submit" => element.click(),
                        // FIXME: file and color not opening.
                        "color" => (),
                        "file" => {
                            self.model.activated_file_input = Some(input_element);
                            return FileInput as i32
                        },
                        _ => {
                            element.focus();
                            return GoInInsertMode as i32;
                        },
                    }
                }
            }
        }
        else if element.is::<DOMHTMLTextAreaElement>() {
            element.focus();
            return GoInInsertMode as i32;
        }
        else if element.is::<DOMHTMLSelectElement>() {
            if element.get_attribute("multiple").is_some() {
                element.focus();
                return GoInInsertMode as i32;
            }
            else {
                mouse_down(&element.upcast());
            }
        }
        else {
            element.click();
        }
        NoAction as i32
    }

    // Handle the key press event for the hint mode.
    // This hides the hints that are not relevant anymore.
    fn enter_hint_key(&mut self, key: char) {
        self.model.hint_keys.push(key);
        let element = self.model.hint_map.get(&self.model.hint_keys)
            .and_then(|element| element.clone().downcast::<DOMHTMLElement>().ok());
        // If no element is found, hide the unrelevant hints.
        if element.is_some() {
            self.send(ClickHintElement());
        }
        else {
            let document = get_page!(self)
                .and_then(|page| page.get_dom_document());
            if let Some(document) = document {
                let all_hidden = hide_unrelevant_hints(&document, &self.model.hint_keys);
                if all_hidden {
                    self.model.hint_keys.clear();
                    show_all_hints(&document);
                }
            }
        }
    }

    // Focus the first input element.
    fn focus_input(&mut self) {
        let document = get_page!(self)
            .and_then(|page| page.get_dom_document());
        if let Some(document) = document {
            let tag_names = ["input", "textarea"];
            for tag_name in &tag_names {
                let iter = ElementIter::new(document.get_elements_by_tag_name(tag_name));
                for element in iter {
                    let tabindex = element.get_attribute("tabindex");
                    if !is_hidden(&document, &element) && is_enabled(&element) && is_text_input(&element)
                        && tabindex != Some("-1".to_string())
                    {
                        element.focus();
                        self.send(EnterInsertMode());
                    }
                }
            }
        }
    }

    // Hide all the hints.
    fn hide_hints(&self) -> () {
        let page = get_page!(self);
        let elements = page.as_ref()
            .and_then(|page| page.get_dom_document())
            .and_then(|document| document.get_element_by_id(HINTS_ID))
            .and_then(|hints| page.as_ref().and_then(|page| get_body(page).map(|body| (hints, body))));
        if let Some((hints, body)) = elements {
            body.remove_child(&hints).ok();
        }
    }

    fn hover(&mut self, element: DOMHTMLElement) -> i32 {
        if let Some(ref element) = self.model.last_hovered_element {
            mouse_out(element);
        }
        self.model.last_hovered_element = Some(element.clone().upcast());
        mouse_over(&element.upcast());
        NoAction as i32
    }

    // FIXME: use one method with two parameters to load the username and the password at the same
    // time.
    // Load the password in the login form.
    fn load_password(&self, password: &str) -> () {
        let document = get_document!(self);
        load_password(&document, password);
    }

    // Load the username in the login form.
    fn load_username(&self, username: &str) -> () {
        let document = get_document!(self);
        load_username(&document, username);
    }

    // Scroll to the bottom of the page.
    fn scroll_bottom(&self) -> () {
        if let Some(page) = get_page!(self) {
            page.scroll_bottom();
        }
    }

    // Scroll by the specified amount of pixels.
    fn scroll_by(&self, pixels: i64) -> () {
        if let Some(page) = get_page!(self) {
            page.scroll_by(pixels);
        }
    }

    // Scroll horizontally by the specified amount of pixels.
    fn scroll_by_x(&self, pixels: i64) -> () {
        if let Some(page) = get_page!(self) {
            page.scroll_by_x(pixels);
        }
    }

    // Scroll to the top of the page.
    fn scroll_top(&self) -> () {
        if let Some(page) = get_page!(self) {
            page.scroll_top();
        }
    }

    // Set the selected file on the input[type="file"].
    fn select_file(&mut self, file: &str) -> () {
        if let Some(ref input_file) = self.model.activated_file_input.take() {
            // FIXME: this is not working.
            input_file.set_value(file);
        }
    }

    // Send a message to the server.
    fn send(&mut self, msg: Message) {
        if let Ok(AsyncSink::Ready) = self.model.writer.start_send(msg) {
            // TODO: log error.
            self.model.writer.poll_complete();
        }
    }

    // Get the username and password from the login form.
    fn send_credentials(&mut self) {
        let mut username = String::new();
        let mut password = String::new();
        let credential = get_page!(self)
            .and_then(|page| page.get_dom_document())
            .and_then(|document| get_credentials(&document));
        if let Some(credential) = credential {
            username = credential.username;
            password = credential.password;
        }
        self.send(Credentials(username, password));
    }

    // Get the page scroll percentage.
    fn send_scroll_percentage(&mut self) {
        let percentage =
            if let Some(page) = get_page!(self) {
                page.scroll_percentage()
            }
            else {
                0
            };
        // TODO: only send the message if the percentage actually changed?
        // TODO: even better: had an even handler for scrolling and only send a message when an
        // actual scroll happened.
        self.send(ScrollPercentage(percentage));
    }

    // Show the hint of elements using the hint characters.
    fn show_hints(&mut self, hint_chars: &str) -> () {
        self.model.hint_keys.clear();
        let page = get_page!(self);
        let body = page.as_ref().and_then(|page| get_body(page));
        let document = page.and_then(|page| page.get_dom_document());
        if let (Some(document), Some(body)) = (document, body) {
            if let Some((hints, hint_map)) = create_hints(&document, hint_chars) {
                self.model.hint_map = hint_map;
                body.append_child(&hints).ok();
            }
        }
    }

    // Submit the login form.
    fn submit_login_form(&self) -> () {
        let document = get_document!(self);
        submit_login_form(&document);
    }
}