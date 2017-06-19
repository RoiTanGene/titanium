/*
 * Copyright (c) 2016-2017 Boucher, Antoni <bouanto@zoho.com>
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

macro_rules! handle_error {
    ($app:ident . $($tt:tt)* ) => {{
        let result = $app.$($tt)*;
        $app.handle_error(result);
    }};
}

mod bookmarks;
mod browser;
mod config;
mod copy_paste;
mod dialog;
mod download;
mod hints;
mod pass_filler;
mod paths;
mod popup;
mod search_engine;
mod server;
mod test_utils;

use std::collections::HashMap;
use std::env;
use std::error;
use std::fmt::{self, Display, Formatter};
use std::process::Command;

use gdk::{EventButton, EventKey, Rectangle, CONTROL_MASK};
use gtk::{self, Inhibit, OrientableExt, WidgetExt};
use gtk::Orientation::Vertical;
use mg::{
    AppClose,
    Completers,
    CompletionViewChange,
    CustomCommand,
    DarkTheme,
    DialogResult,
    Error,
    Info,
    Message,
    Mg,
    ModeChanged,
    Modes,
    SetMode,
    SettingChanged,
    StatusBarItem,
    Text,
    Title,
};
use mg_settings::errors::ErrorKind;
use relm::{EventStream, Relm, Resolver, Update, Widget};
use relm_attributes::widget;
use webkit2gtk::{
    Download,
    FileChooserRequest,
    HitTestResult,
    NavigationAction,
    WebContext,
    WebViewExt,
};
use webkit2gtk::LoadEvent::{self, Finished, Started};
use webkit2gtk::NavigationType::Other;

use bookmarks::BookmarkManager;
use commands::AppCommand;
use commands::AppCommand::*;
use completers::{BookmarkCompleter, FileCompleter};
use config_dir::ConfigDir;
use download_list_view::DownloadListView;
use download_list_view::Msg::{Add, DownloadOriginalDestination};
use message_server::MessageServer;
use pass_manager::PasswordManager;
use popup_manager::PopupManager;
use self::config::default_config;
use self::dialog::handle_script_dialog;
use self::download::find_download_destination;
use self::Msg::*;
use settings::AppSettings;
use settings::AppSettingsVariant::{
    self,
    HintChars,
    HomePage,
};
use webview::WebView;
use webview::Msg::{
    AddScripts,
    AddStylesheets,
    Close,
    EndSearch,
    NewWindow,
    PageFinishSearch,
    PageOpen,
    PagePrint,
    PageScreenshot,
    PageSearch,
    PageSearchNext,
    PageSearchPrevious,
    PageZoomIn,
    PageZoomNormal,
    PageZoomOut,
    SearchBackward,
    SetOpenInNewWindow,
    ShowInspector,
    WebViewSettingChanged,
    ZoomChange,
};

pub const APP_NAME: &'static str = env!("CARGO_PKG_NAME");
const LEFT_BUTTON: u32 = 1;

static MODES: Modes = &[
    ("f", "follow"),
    ("i", "insert"),
];

// TODO: create a custom error type instead of boxing an error.
pub type AppResult<T> = Result<T, Box<error::Error>>;

#[derive(Clone, Copy)]
pub enum FollowMode {
    Click,
    Hover,
}

impl Display for FollowMode {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        let string =
            match *self {
                FollowMode::Click => "click",
                FollowMode::Hover => "hover",
            };
        write!(formatter, "{}", string)
    }
}

pub struct Model {
    bookmark_manager: BookmarkManager,
    client: usize,
    command_text: String,
    config_dir: ConfigDir,
    current_url: String,
    default_search_engine: Option<String>,
    follow_mode: FollowMode,
    has_active_downloads: bool,
    hint_chars: String,
    home_page: Option<String>,
    hovered_link: Option<String>,
    init_url: Option<String>,
    message_server: EventStream<<MessageServer as Update>::Msg>,
    mode: String,
    relm: Relm<App>,
    popup_manager: PopupManager,
    scroll_text: String,
    search_engines: HashMap<String, String>,
    title: String,
}

#[derive(Msg)]
pub enum Msg {
    Action(i32),
    AppSetMode(String),
    AppSettingChanged(AppSettingsVariant),
    ButtonRelease(EventButton, Resolver<Inhibit>),
    ClickElement,
    Create(NavigationAction),
    Command(AppCommand),
    CommandText(String),
    DecideDownloadDestination(Download, String),
    DownloadDestination(DialogResult, Download, String),
    EmitScrolledEvent,
    FileChooser(FileChooserRequest, Resolver<bool>),
    GoToInsertMode,
    KeyPress(EventKey, Resolver<Inhibit>),
    LoadChanged(LoadEvent),
    MouseTargetChanged(HitTestResult),
    PopupDecision(Option<String>, String),
    Scroll(i64),
    ShowZoom(i32),
    TitleChanged,
    TryClose,
    UriChanged,
    WebProcessCrashed,
    WebViewClose,
}

#[widget]
impl Widget for App {
    fn init_view(&mut self) {
        handle_error!(self.model.bookmark_manager.create_tables());

        handle_error!(self.model.bookmark_manager.connect(App::bookmark_path(&self.model.config_dir)));
        handle_error!(self.model.popup_manager.load());

        if let Some(ref url) = self.model.init_url {
            self.webview.emit(PageOpen(url.clone()));
        }

        let mg = self.mg.stream().clone();
        connect!(self.model.relm, self.webview.widget(), connect_script_dialog(_, script_dialog),
            return handle_script_dialog(script_dialog, &mg));

        if let Some(context) = self.get_webview_context() {
            let stream = self.model.relm.stream().clone();
            let list_stream = self.download_list_view.stream().clone();
            connect!(context, connect_download_started(_, download), self.download_list_view, {
                let stream = stream.clone();
                let list_stream = list_stream.clone();
                download.connect_decide_destination(move |download, suggested_filename| {
                    let destination = find_download_destination(suggested_filename);
                    download.set_destination(&format!("file://{}", destination));
                    stream.emit(DecideDownloadDestination(download.clone(), suggested_filename.to_string()));
                    list_stream.emit(DownloadOriginalDestination(download.clone(), destination));
                    true
                });
                Add(download.clone())
            });
        }

        // TODO
        //app.create_password_keyring();
        self.create_variables();

        self.listen_messages();
    }

    fn model(relm: &Relm<Self>, (init_url, config_dir): (Option<String>, Option<String>)) -> Model {
        let config_dir = ConfigDir::new(&config_dir).unwrap();
        // TODO: better error handling.
        let (whitelist, blacklist) = App::popup_path(&config_dir);
        let whitelist = whitelist.expect("cannot create configuration directory");
        let blacklist = blacklist.expect("cannot create configuration directory");
        let popup_manager = PopupManager::new((whitelist, blacklist));
        Model {
            bookmark_manager: BookmarkManager::new(),
            client: 0, // TODO: real client ID.
            command_text: String::new(),
            config_dir,
            current_url: String::new(),
            default_search_engine: None,
            follow_mode: FollowMode::Click,
            has_active_downloads: false,
            hint_chars: "hjklasdfgyuiopqwertnmzxcvb".to_string(),
            home_page: None,
            hovered_link: None,
            init_url,
            message_server: MessageServer::new().unwrap(), // TODO: handle error elsewhere.
            mode: "normal".to_string(),
            relm: relm.clone(),
            popup_manager,
            scroll_text: "[top]".to_string(),
            search_engines: HashMap::new(),
            title: APP_NAME.to_string(),
        }
    }

    /// Set the title of the window as the progress and the web page title.
    fn set_title(&mut self) {
        let progress = (self.webview.widget().get_estimated_load_progress() * 100.0) as i32;
        if progress == 100 {
            self.set_title_without_progress();
        }
        else {
            let title = self.get_title();
            self.model.title = format!("[{}%] {}{}", progress, title, APP_NAME);
        }
    }

    /// Set the title of the window as the web page title or url.
    fn set_title_without_progress(&mut self) {
        let title = self.get_title();
        self.model.title = format!("{}{}", title, APP_NAME);
    }

    /// Show the scroll percentage.
    fn show_scroll(&mut self, scroll_percentage: i64) {
        self.model.scroll_text =
            match scroll_percentage {
                -1 => "[all]".to_string(),
                0 => "[top]".to_string(),
                100 => "[bot]".to_string(),
                _ => format!("[{}%]", scroll_percentage),
            };
    }

    fn update(&mut self, event: Msg) {
        match event {
            Action(action) => self.activate_action(action),
            AppSetMode(mode) => self.model.mode = mode,
            AppSettingChanged(setting) => self.setting_changed(setting),
            ButtonRelease(event, resolver) => self.handle_button_release(event, resolver),
            ClickElement => self.click_hint_element(),
            Create(navigation_action) => self.handle_create(navigation_action),
            Command(ref command) => self.handle_command(command),
            CommandText(text) => self.model.command_text = text,
            DecideDownloadDestination(download, suggested_filename) =>
                self.download_input(download, suggested_filename),
            DownloadDestination(destination, download, suggested_filename) =>
                self.download_destination_chosen(destination, download, suggested_filename),
            EmitScrolledEvent => self.emit_scrolled_event(),
            FileChooser(file_chooser_request, resolver) => self.handle_file_chooser(file_chooser_request, resolver),
            GoToInsertMode => self.go_in_insert_mode(),
            KeyPress(event_key, resolver) => self.handle_key_press(event_key, resolver),
            LoadChanged(load_event) => self.handle_load_changed(load_event),
            MouseTargetChanged(hit_test_result) => self.mouse_target_changed(hit_test_result),
            PopupDecision(answer, url) => self.handle_answer(answer.as_ref().map(|str| str.as_str()), &url),
            Scroll(scroll_percentage) => self.show_scroll(scroll_percentage),
            ShowZoom(level) => self.show_zoom(level),
            TitleChanged => self.set_title(),
            TryClose => self.quit(),
            UriChanged => self.uri_changed(),
            WebProcessCrashed => self.web_process_crashed(),
            WebViewClose => gtk::main_quit(),
        }
    }

    /// Handle the URI changed event.
    fn uri_changed(&mut self) {
        if let Some(url) = self.webview.widget().get_uri() {
            self.model.current_url = url;
        }
    }

    view! {
        #[name="mg"]
        Mg<AppCommand, AppSettings>((MODES, self.model.config_dir.config_file("config"),
            Some(self.model.config_dir.config_home()), default_config(&self.model.config_dir)))
        {
            Completers: hash! {
                "file" => Box::new(FileCompleter::new()),
                "open" => Box::new(BookmarkCompleter::new("open")),
                "win-open" => Box::new(BookmarkCompleter::new("win-open")),
            },
            DarkTheme: true,
            Title: self.model.title.clone(),
            gtk::Box {
                orientation: Vertical,
                #[name="download_list_view"]
                DownloadListView {
                },
                #[name="webview"]
                WebView(self.model.config_dir.clone()) {
                    Close => WebViewClose,
                    NewWindow(ref url) => Command(WinOpen(url.clone())),
                    ZoomChange(level) => ShowZoom(level),
                    button_release_event(_, event) => async ButtonRelease(event.clone()),
                    create(_, action) => (Create(action.clone()), None),
                    // Emit the scroll event whenever the view is drawn.
                    draw(_, _) => (EmitScrolledEvent, Inhibit(false)),
                    load_changed(_, load_event) => LoadChanged(load_event),
                    mouse_target_changed(_, hit_test_result, _) => MouseTargetChanged(hit_test_result.clone()),
                    resource_load_started(_, _, _) => TitleChanged,
                    run_file_chooser(_, file_chooser_request) => async FileChooser(file_chooser_request.clone()),
                    title_changed() => TitleChanged,
                    uri_changed() => UriChanged,
                    web_process_crashed => (WebProcessCrashed, false),
                },
            },
            #[name="scroll_label"]
            StatusBarItem {
                Text: self.model.scroll_text.clone(),
            },
            StatusBarItem {
                Text: self.model.current_url.clone(),
            },
            AppClose => TryClose,
            CompletionViewChange(ref completion) => CommandText(completion.clone()),
            CustomCommand(ref command) => Command(command.clone()),
            ModeChanged(ref mode) => AppSetMode(mode.clone()),
            SettingChanged(ref setting) => AppSettingChanged(setting.clone()),
            key_press_event(_, event_key) => async KeyPress(event_key.clone()),
        }
    }
}

impl App {
    /// Show an error from a string.
    pub fn error(&self, error: &str) {
        self.mg.emit(Error(ErrorKind::Msg(error.to_string()).into()));
    }

    /// Give the focus to the webview.
    fn focus_webview(&self) {
        self.webview.widget().grab_focus();
    }

    /// Get the size of the webview.
    fn get_webview_allocation(&self) -> Rectangle {
        self.webview.widget().get_allocation()
    }

    /// Get the title or the url if there are no title.
    fn get_title(&self) -> String {
        let title = self.webview.widget().get_title()
            .or(self.webview.widget().get_uri())
            .unwrap_or_default();
        if title.is_empty() {
            String::new()
        }
        else {
            format!("{} - ", title)
        }
    }

    fn get_webview_context(&self) -> Option<WebContext> {
        let context = self.webview.widget().get_context();
        if context.is_none() {
            self.error("Cannot retrieve web view context");
        }
        context
    }

    /// Handle the button release event to open in new window when using Ctrl-click.
    fn handle_button_release(&mut self, event: EventButton, mut resolver: Resolver<Inhibit>) {
        if event.get_button() == LEFT_BUTTON && event.get_state().contains(CONTROL_MASK) {
            if let Some(url) = self.model.hovered_link.clone() {
                self.open_in_new_window_handling_error(&url);
                resolver.resolve(Inhibit(true));
            }
        }
    }

    /// Handle the command.
    fn handle_command(&mut self, command: &AppCommand) {
        match *command {
            ActivateSelection => handle_error!(self.activate_selection()),
            Back => self.webview.widget().go_back(),
            BackwardSearch(ref input) => {
                self.webview.emit(SearchBackward(true));
                self.webview.emit(PageSearch(input.clone()));
            },
            Bookmark => self.bookmark(),
            BookmarkDel => self.delete_bookmark(),
            BookmarkEditTags => self.edit_bookmark_tags(),
            ClearCache => self.clear_cache(),
            CopyUrl => self.copy_url(),
            DeleteAllCookies => self.delete_all_cookies(),
            DeleteCookies(ref domain) => self.delete_cookies(domain),
            DeleteSelectedBookmark => self.delete_selected_bookmark(),
            FinishSearch => self.webview.emit(PageFinishSearch),
            FocusInput => handle_error!(self.focus_input()),
            Follow => {
                self.model.follow_mode = FollowMode::Click;
                self.webview.emit(SetOpenInNewWindow(false));
                self.mg.emit(SetMode("follow"));
                handle_error!(self.follow_link())
            },
            Forward => self.webview.widget().go_forward(),
            HideHints => self.hide_hints(),
            Hover => {
                self.model.follow_mode = FollowMode::Hover;
                self.mg.emit(SetMode("follow"));
                handle_error!(self.follow_link())
            },
            Insert => self.go_in_insert_mode(),
            Inspector => self.webview.emit(ShowInspector),
            Normal => self.go_in_normal_mode(),
            Open(ref url) => self.open(url),
            PasswordDelete => self.delete_password(),
            PasswordLoad => self.load_password(),
            PasswordSave => self.save_password(),
            PasswordSubmit => self.submit_login_form(),
            PasteUrl => self.paste_url(),
            Print => self.webview.emit(PagePrint),
            Quit => self.quit(),
            Reload => self.webview.widget().reload(),
            ReloadBypassCache => self.webview.widget().reload_bypass_cache(),
            Screenshot(ref path) => self.webview.emit(PageScreenshot(path.clone())),
            ScrollBottom => handle_error!(self.scroll_bottom()),
            ScrollDown => handle_error!(self.scroll_down_page()),
            ScrollDownHalf => handle_error!(self.scroll_down_half_page()),
            ScrollDownLine => handle_error!(self.scroll_down_line()),
            ScrollLeft => handle_error!(self.scroll_left()),
            ScrollRight => handle_error!(self.scroll_right()),
            ScrollTop => handle_error!(self.scroll_top()),
            ScrollUp => handle_error!(self.scroll_up_page()),
            ScrollUpHalf => handle_error!(self.scroll_up_half_page()),
            ScrollUpLine => handle_error!(self.scroll_up_line()),
            Search(ref input) => {
                self.webview.emit(SearchBackward(false));
                self.webview.emit(PageSearch(input.clone()));
            },
            SearchEngine(ref args) => self.add_search_engine(args),
            SearchNext => self.webview.emit(PageSearchNext),
            SearchPrevious => self.webview.emit(PageSearchPrevious),
            Stop => self.webview.widget().stop_loading(),
            WinFollow => {
                self.model.follow_mode = FollowMode::Click;
                self.webview.emit(SetOpenInNewWindow(true));
                self.mg.emit(SetMode("follow"));
                handle_error!(self.follow_link())
            },
            WinOpen(ref url) => self.open_in_new_window_handling_error(url),
            WinPasteUrl => self.win_paste_url(),
            ZoomIn => self.zoom_in(),
            ZoomNormal => self.zoom_normal(),
            ZoomOut => self.zoom_out(),
        }
    }

    fn go_in_insert_mode(&mut self) {
        self.mg.emit(SetMode("insert"));
    }

    fn go_in_normal_mode(&mut self) {
        self.mg.emit(SetMode("normal"));
    }

    /// Handle create window.
    fn handle_create(&mut self, action: NavigationAction) {
        if let Some(request) = action.get_request() {
            if let Some(url) = request.get_uri() {
                if action.get_navigation_type() == Other && !self.model.popup_manager.is_whitelisted(&url) {
                    self.handle_popup(url);
                }
                else {
                    self.open_in_new_window_handling_error(&url);
                }
            }
        }
    }

    /// Show an error in the result is an error.
    fn handle_error(&self, error: AppResult<()>) {
        if let Err(error) = error {
            self.show_error(error);
        }
    }

    /// Handle the key press event.
    fn handle_key_press(&mut self, event_key: EventKey, mut resolver: Resolver<Inhibit>) {
        if self.model.mode == "follow" {
            self.handle_follow_key_press(event_key);
            resolver.resolve(Inhibit(true));
        }
    }

    /// Handle the load_changed event.
    /// Show the URL.
    /// Set the window title.
    /// Go back to normal mode.
    fn handle_load_changed(&mut self, load_event: LoadEvent) {
        if load_event == Started {
            self.webview.emit(EndSearch);
            self.webview.emit(AddStylesheets);
            self.webview.emit(AddScripts);

            // Check to mode to avoid going back to normal mode if the user is in command mode.
            if self.model.mode == "insert" || self.model.mode == "follow" {
                self.go_in_normal_mode();
            }
        }

        if load_event == Finished {
            self.set_title_without_progress();
        }
        else {
            self.set_title();
        }
    }

    /// Show an info.
    pub fn info(&self, info: String) {
        self.mg.emit(Info(info));
    }

    /// Handle the mouse target changed event of the webview to show the hovered URL and save it
    /// for use when using Ctrl-click.
    fn mouse_target_changed(&mut self, hit_test_result: HitTestResult) {
        let link = hit_test_result.get_link_uri();
        self.model.hovered_link = link.clone();
        {
            let text = link.unwrap_or_else(String::new);
            self.mg.emit(Message(text));
        }
    }

    /// Open the given URL in the web view.
    fn open(&self, url: &str) {
        let url = self.transform_url(url);
        self.webview.emit(PageOpen(url));
    }

    /// Open the given URL in a new window.
    fn open_in_new_window(&self, url: &str) -> AppResult<()> {
        let url = self.transform_url(url);
        let program = env::args().next().unwrap();
        Command::new(program)
            .arg(url)
            .spawn()?;
        Ok(())
    }

    /// Open the given URL in a new window, showing the error if any.
    fn open_in_new_window_handling_error(&self, url: &str) {
        handle_error!(self.open_in_new_window(url));
    }

    /// Try to close the web view and quit the application.
    fn quit(&self) {
        // Ask for a confirmation before quitting the application when there are active
        // downloads.
        let can_quit =
            // TODO: set this variable on download list view events.
            if self.model.has_active_downloads {
                self.blocking_yes_no_question("There are active downloads. Do you want to quit?")
            }
            else {
                true
            };

        if can_quit {
            self.webview.widget().try_close();
        }
    }

    fn setting_changed(&mut self, setting: AppSettingsVariant) {
        match setting {
            HintChars(chars) => self.model.hint_chars = chars,
            HomePage(url) => {
                if  self.model.init_url.is_none() {
                    self.webview.emit(PageOpen(url.clone()));
                }
                self.model.home_page = Some(url);
            },
            _ => self.webview.emit(WebViewSettingChanged(setting)),
        }
    }

    /// Show an error.
    pub fn show_error(&self, error: Box<error::Error>) {
        self.error(&error.to_string());
    }

    /// Show the zoom level in the status bar.
    fn show_zoom(&self, level: i32) {
        self.info(format!("Zoom level: {}%", level));
    }

    /// Handle the web process crashed event.
    fn web_process_crashed(&mut self) {
        self.error("The web process crashed.");
    }

    /// Open in a new window the url from the system clipboard.
    fn win_paste_url(&self) {
        if let Some(url) = self.get_url_from_clipboard() {
            self.open_in_new_window_handling_error(&url);
        }
    }

    /// Zoom in.
    fn zoom_in(&self) {
        self.webview.emit(PageZoomIn);
    }

    /// Zoom back to 100%.
    fn zoom_normal(&self) {
        self.webview.emit(PageZoomNormal);
    }

    /// Zoom out.
    fn zoom_out(&self) {
        self.webview.emit(PageZoomOut);
    }
}
