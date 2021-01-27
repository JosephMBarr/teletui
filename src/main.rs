extern crate chrono;
use chrono::prelude::*;
mod event;
use crossbeam::thread;
use event::{Event, Events};
use rtdlib::types::*;
use rtdlib::Tdlib;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Error, ErrorKind};
use std::sync::{mpsc, Arc, Mutex};
use std::vec;
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Corner, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
// Seconds to wait for message form Tdlib
const TIMEOUT: f64 = 0.5;

// TUI box margin
const MARGIN: u16 = 1;

const DEBUG_LEVEL: i64 = 0;
const DO_DEBUG: bool = false;
const CODE_ARG: &str = "--code=";
const NO_CODE_PROVIDED: &str = "Please re-run with --code={{code}}";
const COLORS: [Color; 13] = [
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
];

enum MsgCode {
    Exit,
}

#[derive(Clone)]
enum InputMode {
    Normal,
    Insert,
}

// TUI Blocks
enum TBlocks {
    ChatList,
    CurrChat,
    Input,
}

// The whole application
#[derive(Clone)]
struct App {
    curr_mode: InputMode,

    // Queue of requests outgoing to Tdlib
    outgoing_queue: Arc<Mutex<VecDeque<String>>>,
    users: Arc<Mutex<HashMap<i64, TUser>>>,
    basic_groups: Arc<Mutex<HashMap<i64, TBasicGroup>>>,
    chat_list: Chats,
    input_box: InputBox,
}
impl App {
    fn new() -> App {
        App {
            curr_mode: InputMode::Normal,
            outgoing_queue: Arc::new(Mutex::new(VecDeque::new())),
            users: Arc::new(Mutex::new(HashMap::new())),
            basic_groups: Arc::new(Mutex::new(HashMap::new())),
            chat_list: Chats::new("Chats"),
            input_box: InputBox::new("Input"),
        }
    }
}

// A wrapper for Tdlib's Basic Group
struct TBasicGroup {
    g: BasicGroup,
    full_info: BasicGroupFullInfo,
}

// The message input box
#[derive(Clone)]
struct InputBox {
    input_str: String,

    // Box title
    name: &'static str,
}

// The box containing the list of chats
#[derive(Clone)]
struct Chats {
    // Vector containing each chat
    chat_vec: Arc<Mutex<Vec<TChat>>>,

    // Title associated with block
    name: &'static str,

    // Index within chat_vec of currently selected chat
    selected_index: Arc<Mutex<usize>>,
}

// A wrapper for Tdlib User with extra information
#[derive(Clone)]
struct TUser {
    u: User,

    // Color of users name in chat; calculated to be as globally unique as possible
    color: Color,
    full_info: UserFullInfo,
    status: UserStatus,
}

// A wrapper for Tdlib Chat with extra information
#[derive(Clone)]
struct TChat {
    // History of messages in this chat
    history: Arc<Mutex<Vec<Message>>>,

    // The relevant chat
    chat: Chat,

    // Whether chat has reached the end of history i.e. no messages left to retrieve
    end_of_history: bool,

    // Starting message id of current request, if there is one
    // Used to prevent redundant requests
    retrieving: i64,

    // Number of messages currently displayed on screen
    num_onscreen: usize,

    // Index (within history) of the message at the bottom of the screen
    bottom_index: usize,

    // Timestamp of most recent message in chat
    last_msg_date: Arc<Mutex<i64>>,
}
impl App {}

impl TChat {
    fn set_last_msg_date(&mut self, d: i64) {
        *self.last_msg_date.lock().unwrap() = d;
    }
    // Retrieve history of messages in chat, starting at message with id `start_id`,
    // retrieving up to `limit` messages, and placing the final request in `queue`, which
    // is generally the app output queue
    fn retrieve_history(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        start_id: i64,
        limit: i64,
    ) {
        if self.retrieving == start_id {
            return;
        }
        self.retrieving = start_id;
        let chat_history_req = GetChatHistory::builder()
            .chat_id(self.chat.id())
            .from_message_id(start_id)
            .limit(limit)
            .only_local(false)
            .build();

        queue
            .lock()
            .unwrap()
            .push_back(chat_history_req.to_json().unwrap());
    }

    // Returns the id of the oldest message in history. If there is no history yet, return 0.
    // Passing 0 as the id to the message request tells Tdlib to get the newest messages
    fn get_oldest_id(&self) -> i64 {
        let hc = self.history.lock().unwrap();
        if hc.len() == 0 {
            return 0;
        }
        hc[hc.len() - 1].id().to_string().parse::<i64>().unwrap()
    }

    // Create a TChat from a JSON string of a Tdlib Chat
    fn from_json(j: String) -> TChat {
        TChat {
            chat: Chat::from_json(j).unwrap(),
            history: Arc::new(Mutex::new(Vec::new())),
            end_of_history: false,
            retrieving: -1,
            num_onscreen: 0,
            bottom_index: 0,

            last_msg_date: Arc::new(Mutex::new(-1)),
        }
    }
}

// TUI block
trait TBlock {
    fn new(name: &'static str) -> Self;

    fn scroll_down(&mut self) {}
    fn scroll_up(&mut self) {}
    fn page_down(&mut self) {}
    fn page_up(&mut self) {}
    fn go_to_bottom(&mut self) {}
    fn get_len(&self) -> usize {
        0
    }
    fn handle_input_insert(
        &mut self,
        _queue: &Arc<Mutex<VecDeque<String>>>,
        _input: &termion::event::Key,
        _cur_chat_id: i64,
    ) {
    }

    // Handle input in normal mode. Generally use classic Vi(m) keybinds
    fn handle_input_normal(
        &mut self,
        _queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
    ) {
        match input {
            Key::Char('j') => self.scroll_down(),
            Key::Char('k') => self.scroll_up(),
            Key::Char('G') => self.go_to_bottom(),
            Key::Ctrl('f') => self.page_down(),
            Key::Ctrl('b') => self.page_up(),
            _ => {}
        }
    }
}

// List of chats
impl Chats {
    // Returns chat having given ID, if it exists
    fn get_chat_by_id(&mut self, chat_id: i64) -> Option<TChat> {
        for chat in self.chat_vec.clone().lock().unwrap().iter() {
            if chat.chat.id() == chat_id {
                return Some(chat.clone());
            }
        }
        None
    }
    fn get_chat_id_by_index(&self, i: usize) -> i64 {
        return self.chat_vec.lock().unwrap().get(i).unwrap().chat.id();
    }
    fn set_selected_index(&mut self, i: usize) {
        *self.selected_index.lock().unwrap() = i;
    }

    fn selected_index(&self) -> usize {
        return *self.selected_index.lock().unwrap();
    }

    fn sort(&mut self) {
        let id_of_selected = self.get_chat_id_by_index(self.selected_index());
        self.chat_vec.lock().unwrap().sort_by(|a, b| {
            b.last_msg_date
                .lock()
                .unwrap()
                .cmp(&a.last_msg_date.lock().unwrap())
        });
        let new_id_of_selected = self.get_chat_id_by_index(self.selected_index());

        if id_of_selected != new_id_of_selected {
            for (i, c) in self.chat_vec.lock().unwrap().iter().enumerate() {
                if c.chat.id() == id_of_selected {
                    *self.selected_index.lock().unwrap() = i as usize;
                    break;
                }
            }
        }
    }
}

// Message input box
impl InputBox {
    // Creates message to be sent to chat, using passed in chat ID and the contents of
    // the input string
    fn send_message(&mut self, cur_chat_id: i64, queue: &Arc<Mutex<VecDeque<String>>>) {
        let msg = InputMessageContent::InputMessageText(
            InputMessageText::builder()
                .text(FormattedText::builder().text(self.input_str.clone()))
                .build(),
        );
        let req = SendMessage::builder()
            .chat_id(cur_chat_id)
            .input_message_content(msg)
            .build();
        queue.lock().unwrap().push_back(req.to_json().unwrap());
        self.input_str.clear();
    }
}

impl TBlock for InputBox {
    fn new(name: &'static str) -> InputBox {
        InputBox {
            input_str: String::new(),
            name,
        }
    }
    fn handle_input_insert(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
        cur_chat_id: i64,
    ) {
        match input {
            Key::Char('\n') => {
                self.send_message(cur_chat_id, queue);
            }

            // Add unremarkable character to input string
            Key::Char(input) => {
                self.input_str.push(*input);
            }
            Key::Backspace => {
                self.input_str.pop();
            }

            _ => {}
        }
    }
}
impl TBlock for Chats {
    fn new(name: &'static str) -> Chats {
        Chats {
            chat_vec: Arc::new(Mutex::new(Vec::new())),
            selected_index: Arc::new(Mutex::new(0)),
            name,
        }
    }

    // Get number of chats
    fn get_len(&self) -> usize {
        return self.chat_vec.lock().unwrap().len();
    }

    fn scroll_up(&mut self) {
        let mut selected_index = self.selected_index();

        // Wrap around when going over top
        if selected_index == 0 {
            selected_index = self.get_len() - 1;
        }
        // Go up by one chat
        selected_index = (selected_index + 1) % self.get_len();

        self.set_selected_index(selected_index);
    }

    fn scroll_down(&mut self) {
        // Wrap around to top of list
        self.set_selected_index((self.selected_index() + 1) % self.get_len());
    }
    fn page_down(&mut self) {}
    fn page_up(&mut self) {}
}

impl TBlock for TChat {
    fn new(_name: &'static str) -> TChat {
        TChat {
            chat: Chat::builder().build(),
            history: Arc::new(Mutex::new(Vec::new())),
            end_of_history: false,
            retrieving: -1,
            num_onscreen: 0,
            bottom_index: 0,
            last_msg_date: Arc::new(Mutex::new(-1)),
        }
    }
    // Go all the way to the bottom (e.g. newest message)
    fn go_to_bottom(&mut self) {
        self.bottom_index = 0;
    }

    fn get_len(&self) -> usize {
        return self.history.lock().unwrap().len();
    }

    // Scroll up such that the topmost message becomes the bottom one
    fn page_up(&mut self) {
        self.bottom_index += self.num_onscreen;
    }
    //TODO: fiddle with scrolling off-by-one
    fn page_down(&mut self) {
        // If the bottom message is also the newest one, make sure it's flush
        // with the bottom of the box, as to avoid blank gaps at bottom
        if (self.bottom_index as i64 - self.num_onscreen as i64) <= 0 {
            self.bottom_index = 0;
            return;
        }

        // Reduce bottom index by page height
        self.bottom_index -= self.num_onscreen;
    }
    fn scroll_up(&mut self) {
        if self.bottom_index + self.num_onscreen < self.get_len() {
            self.bottom_index += 1;
        }
    }
    fn scroll_down(&mut self) {
        // Use same safeguard as paging down to ensure bottom message is always flush with
        // bottom of box
        if self.bottom_index as i64 <= 0 {
            self.bottom_index = 0;
            return;
        }
        self.bottom_index -= 1;
    }
}

fn main() {
    // Telegram API access object
    let tdlib = Tdlib::new();

    // Set verbosity level to handle volume of debug output
    let set_verbosity_level = SetLogVerbosityLevel::builder()
        .new_verbosity_level(DEBUG_LEVEL)
        .build();
    tdlib.send(&set_verbosity_level.to_json().unwrap());

    // Set up cross-thread communication
    let (tx_ui, rx_td) = mpsc::channel::<MsgCode>();
    let (tx_td, rx_ui) = mpsc::channel::<MsgCode>();

    // Start parallel threads, one for UI, the other for managing requests with Tdlib
    thread::scope(|s| {
        let mut app = App::new();

        // Create an Arc reference to pass into request (receiving) thread
        let mut rec_app = app.clone();
        let _rec_thread = s.spawn(move |_| {
            // Spawn thread for managing requests
            td_thread(&tdlib, &mut rec_app, tx_td, rx_td);
        });

        // Spawn UI thread
        ui_thread(&mut app, tx_ui, rx_ui).unwrap();
    })
    .unwrap();
}

// Read in API information and user phone number from file
fn read_info() -> io::Result<(i64, String, String)> {
    let file = File::open("info.txt")?;
    let mut reader = BufReader::new(file);

    let mut api_id = String::new();
    let mut api_hash = String::new();
    let mut phone_number = String::new();

    reader.read_line(&mut api_id)?;
    reader.read_line(&mut api_hash)?;
    reader.read_line(&mut phone_number)?;

    // Remove whitespace and parse ID as int, as required by requests
    let api_id: i64 = api_id.trim().parse().unwrap();

    Ok((
        api_id,
        api_hash.trim().to_string(),
        phone_number.trim().to_string(),
    ))
}

// Get list of user's chats
fn get_chat_list(tdlib: &Tdlib) {
    let chat_list = ChatList::default();
    let chat_list_req = GetChats::builder()
        .chat_list(chat_list)
        .offset_order(0)
        .offset_chat_id(0)
        .limit(255)
        .build();
    tdlib.send(&chat_list_req.to_json().unwrap());
}

// Initialization parameters for Tdlib
fn send_tdlib_parameters(tdlib: &Tdlib, api_id: i64, api_hash: &str) {
    let set_tdlib_parameters = SetTdlibParameters::builder()
        .parameters(
            TdlibParameters::builder()
                // Don't use test data, communicate with actual Telegram
                .use_test_dc(false)
                .database_directory("/tmp/td")
                .files_directory("/tmp/td")
                .use_file_database(false)
                .api_id(api_id)
                .api_hash(api_hash)
                .system_language_code("en")
                .device_model("computer")
                .application_version("0.0.1")
                .build(),
        )
        .build();

    tdlib.send(&set_tdlib_parameters.to_json().unwrap());
}

// Check database encryption key with Telegram
// TODO: use actual key
fn send_check_encryption_key(tdlib: &Tdlib) {
    let check_enc_key = CheckDatabaseEncryptionKey::builder().build();
    tdlib.send(&check_enc_key.to_json().unwrap());
}

// Send phone number to Tdlib to connect app with account
fn send_phone_parameters(tdlib: &Tdlib, phone_number: &str) {
    let phone_parameters = SetAuthenticationPhoneNumber::builder()
        .phone_number(phone_number)
        .settings(PhoneNumberAuthenticationSettings::builder().build())
        .build();

    tdlib.send(&phone_parameters.to_json().unwrap());
}

// Send agreement to terms of service
fn _send_registration(tdlib: &Tdlib, first_name: &str, last_name: &str) {
    let reg = RegisterUser::builder()
        .first_name(first_name)
        .first_name(last_name)
        .build();

    tdlib.send(&reg.to_json().unwrap());
}

// Driver for Tdlib communication
fn td_thread(tdlib: &Tdlib, app: &mut App, tx: mpsc::Sender<MsgCode>, rx: mpsc::Receiver<MsgCode>) {
    let (api_id, api_hash, phone_number) = read_info().unwrap();
    // Wait for message for `TIMEOUT` seconds
    loop {
        // Check for cross-thread messages
        if let Ok(c) = rx.try_recv() {
            match c {
                MsgCode::Exit => {
                    return;
                }
            }
        }
        let sz = app.outgoing_queue.lock().unwrap().len();
        // Send each request in queue, in order
        for _ in 0..sz {
            let s = app.outgoing_queue.lock().unwrap().pop_front().unwrap();
            tdlib.send(&s);
        }
        let res = match tdlib.receive(TIMEOUT) {
            Some(r) => r,
            None => continue,
        };
        // Decode request string into an object
        let mut obj: Value = serde_json::from_str(&res).unwrap();
        if DO_DEBUG {
            eprintln!("Received: {:?}", obj.get("@type"));
        }
        //TODO: less string wizardry
        match &detect_td_type(&res).unwrap()[..] {
            // Received any of a number of auth state changes
            "updateAuthorizationState" => {
                let astate =
                    AuthorizationState::from_json(obj["authorization_state"].to_string()).unwrap();
                match astate {
                    // Authorization complete, get list of chats
                    AuthorizationState::Ready(_) => {
                        //AuthorizationState::Ready => {
                        //TODO: store auth credentials
                        get_chat_list(&tdlib);
                    }
                    // Initial setup request
                    AuthorizationState::WaitTdlibParameters(_) => {
                        send_tdlib_parameters(&tdlib, api_id, &api_hash);
                    }

                    // Send Tdlib database encryption key
                    AuthorizationState::WaitEncryptionKey(_) => send_check_encryption_key(&tdlib),

                    // Tdlib is waiting for phone number
                    AuthorizationState::WaitPhoneNumber(_) => {
                        send_phone_parameters(&tdlib, &phone_number);
                    }

                    // Tdlib is awaiting authorization code that was sent
                    // to user via Telegram, SMS, or otherwise
                    AuthorizationState::WaitCode(_) => {
                        // Get code argument from command line args
                        let input_code = match get_arg(CODE_ARG) {
                            Some(c) => c,
                            None => {
                                // Code was needed but not provided,
                                // so exit and tell user to run again, providing code
                                eprintln!("{}", NO_CODE_PROVIDED);
                                tx.send(MsgCode::Exit).unwrap();
                                return;
                            }
                        };

                        // Check provided auth code against Tdlib's expectation
                        let check_auth_code = CheckAuthenticationCode::builder()
                            .code(input_code.trim())
                            .build();

                        tdlib.send(&check_auth_code.to_json().unwrap());
                    }
                    _ => {
                        eprintln!("unhandled auth case!: {}", astate.to_json().unwrap());
                    }
                }
            }

            // Received user information. Can be new or an update to an existing
            "updateUser" => {
                let num_users = app.users.lock().unwrap().len();
                let uid = obj["user"]["id"].as_i64().unwrap();
                // Create TUser, parsing JSON as a User and determining name color,
                // or update if already exists
                app.users
                    .lock()
                    .unwrap()
                    .entry(uid)
                    .and_modify(|tu| tu.u = User::from_json(obj["user"].to_string()).unwrap())
                    .or_insert(TUser {
                        u: User::from_json(obj["user"].to_string()).unwrap(),
                        // Calculate the next color to use, maintaining maximum variety
                        color: COLORS[num_users % COLORS.len()],
                        full_info: UserFullInfo::builder().build(),
                        status: UserStatus::from_json(obj["user"]["status"].to_string()).unwrap(),
                    });
            }

            // Received an update to users status (online/offline/etc.)
            "updateUserStatus" => {
                let uid = obj["user_id"].as_i64().unwrap();
                app.users.lock().unwrap().entry(uid).and_modify(|tu| {
                    tu.status = UserStatus::from_json(obj["status"].to_string()).unwrap();
                });
            }

            // Received information about a basic group
            "updateBasicGroup" => {
                // Parse JSON as Basic Group and insert (or update) to HashMap
                let new_group = TBasicGroup {
                    g: BasicGroup::from_json(obj["basic_group"].to_string()).unwrap(),
                    full_info: BasicGroupFullInfo::default(),
                };
                app.basic_groups
                    .lock()
                    .unwrap()
                    .insert(new_group.g.id(), new_group);
            }
            // Received full information about a basic group
            "updateBasicGroupFullInfo" => {
                // Parse JSON as Basic Group and insert (or update) to HashMap
                app.basic_groups
                    .lock()
                    .unwrap()
                    .entry(obj["basic_group_id"].as_i64().unwrap())
                    .and_modify(|bgf| {
                        bgf.full_info =
                            BasicGroupFullInfo::from_json(obj["basic_group_full_info"].to_string())
                                .unwrap()
                    });
            }

            // Received information about a chat of which we've not heard before
            "updateNewChat" => {
                let new_chat = &mut obj["chat"];

                // Add attributes to new_chat that are expected by rtdlib,
                // but not provided by the API
                new_chat["order"] = serde_json::from_str("1").unwrap();
                new_chat["is_pinned"] = serde_json::from_value(json!(false)).unwrap();
                new_chat["is_sponsored"] = serde_json::from_value(json!(false)).unwrap();
                new_chat["pinned_message_id"] = serde_json::from_value(json!(0)).unwrap();

                // Add TChat to chat list
                let tchat = TChat::from_json(new_chat.to_string());
                app.chat_list.chat_vec.lock().unwrap().push(tchat);
            }

            "updateChatLastMessage" => {
                let chat_id = obj["chat_id"].as_i64().unwrap();
                app.chat_list
                    .get_chat_by_id(chat_id)
                    .unwrap()
                    .set_last_msg_date(obj["last_message"]["date"].as_i64().unwrap());
                app.chat_list.sort();
            }

            // Received information about a message of which we've not heard before
            "updateNewMessage" => {
                let msg = &mut obj["message"];
                let chat_id = match msg["chat_id"].as_i64() {
                    Some(ok) => ok,
                    None => panic!("Couldn't get id: {}", msg),
                };

                // Determine the chat to which message belongs
                let cur_chat = &mut app.chat_list.get_chat_by_id(chat_id).unwrap();
                let mut cur_chat_history = cur_chat.history.lock().unwrap();

                // Parse message into rtdlib::Message type
                let cur_msg = parse_msg(msg, chat_id);
                // Place at start, rather than push to end
                cur_chat_history.insert(0, cur_msg);
            }

            // Received a list of messages, initiated by GetChatHistory call
            "messages" => {
                eprintln!("messages is {}", obj);
                let msg_count = obj["total_count"].as_u64().unwrap();
                let msg_list = &mut obj["messages"];
                let chat_id = match msg_list[0]["chat_id"].as_i64() {
                    Some(ok) => ok,
                    None => {
                        eprintln!("Couldn't get id: {}", obj);
                        continue;
                    }
                };

                // If received at least one message, insert into history
                if msg_count > 0 {
                    let cur_chat = &mut app.chat_list.get_chat_by_id(chat_id).unwrap();
                    let mut cur_chat_history = cur_chat.history.lock().unwrap();
                    for cur_msg in msg_list.as_array_mut().unwrap() {
                        let cur_msg = parse_msg(cur_msg, chat_id);
                        cur_chat_history.push(cur_msg);
                    }
                    cur_chat.retrieving = -1;
                }
            }
            "error" => {
                let msg = obj["message"].as_str().unwrap();
                let mut is_fatal = false;
                let error_msg = match msg {
                    "PHONE_CODE_INVALID" => {
                        is_fatal = true;
                        "Incorrect code. Please try again."
                    }
                    _ => msg,
                };
                eprintln!("{}", error_msg);
                if is_fatal {
                    tx.send(MsgCode::Exit).unwrap();
                    return;
                }
            }
            _ => {
                eprintln!("Unhandled message: {}", obj);
            }
        }
    }
}

fn ui_thread(
    app: &mut App,
    tx: mpsc::Sender<MsgCode>,
    rx: mpsc::Receiver<MsgCode>,
) -> Result<(), std::io::Error> {
    let selected_style: Style = Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD);
    let unselected_style: Style = Style::default()
        .fg(Color::White)
        .remove_modifier(Modifier::BOLD);
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();
    let mut selected_block = TBlocks::ChatList;

    terminal.clear()?;
    let mut chat_box_height = 0;
    let mut chat_box_width: usize = 0;
    loop {
        if let Ok(c) = rx.try_recv() {
            match c {
                MsgCode::Exit => return Ok(()),
            }
        }

        terminal.draw(|f| {
            let size = f.size();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(MARGIN)
                .constraints([Constraint::Percentage(85), Constraint::Percentage(15)].as_ref())
                .split(size);
            let chat_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(MARGIN)
                .constraints([Constraint::Percentage(20), Constraint::Percentage(80)].as_ref())
                .split(chunks[0]);
            let mut chat_titles = vec::Vec::new();
            let ui_users = app.users.lock().unwrap();
            let ui_basic_groups = app.basic_groups.lock().unwrap();
            chat_box_height = (chat_chunks[1].bottom() - chat_chunks[1].top() - 2 * MARGIN).into();
            chat_box_width = (chat_chunks[1].right() - chat_chunks[1].left() - 2 * MARGIN).into();
            let mut chat_history = vec::Vec::new();
            let mut chat_title = "Current Chat".to_owned();
            for (i, chat) in (app.chat_list.chat_vec)
                .lock()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                let chat_list_item = ListItem::new(Text::from(String::from(chat.chat.title())));
                if app.chat_list.selected_index() != i {
                    // Not selected, style as default and skip ahead to next
                    chat_titles.push(chat_list_item.style(unselected_style));
                    continue;
                }
                let (displayed_msgs, history_height) = build_msg_list(
                    &chat,
                    &ui_users,
                    chat_box_width,
                    chat_box_height,
                    &mut chat_history,
                );
                chat_titles.push(chat_list_item.style(selected_style));
                chat.num_onscreen = displayed_msgs;
                //TODO: fix end of history
                let oldest_id = chat.get_oldest_id();
                if (history_height < chat_box_height.into()
                    || chat.bottom_index + 2 * chat.num_onscreen >= chat_history.len())
                    && !chat.end_of_history
                {
                    chat.retrieve_history(
                        &app.outgoing_queue,
                        oldest_id,
                        (chat_box_height * 2) as i64,
                    );
                }
                let extra_info = if chat.chat.type_().is_private() {
                    // Get user and the time they were last seen
                    let recipient_id = chat.chat.type_().as_private().unwrap().user_id();
                    let recipient = ui_users.get(&recipient_id).unwrap();
                    let status = &recipient.status;
                    if status.is_online() {
                        "online".to_string()
                    } else if status.is_offline() {
                        let ts: u64 = status.as_offline().unwrap().was_online() as u64;
                        let d = std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts);
                        let date_time = DateTime::<Local>::from(d);
                        format!("last seen {}", date_time.format("%H:%M on %m/%d"))
                    } else {
                        "unknown".to_string()
                    }
                } else if chat.chat.type_().is_basic_group() {
                    let group_id = chat.chat.type_().as_basic_group().unwrap().basic_group_id();
                    let group = ui_basic_groups.get(&group_id).unwrap();

                    // Count up how many members in chat are online
                    let members_online = group
                        .full_info
                        .members()
                        .into_iter()
                        .filter(|m| {
                            let u = ui_users.get(&m.user_id()).unwrap();
                            u.status.is_online() && u.u.type_().is_regular()
                        })
                        .count();
                    format!(
                        "{} members, {} online",
                        group.g.member_count(),
                        members_online
                    )
                } else {
                    "unknown".to_string()
                };
                chat_title = format!("{}: {}", *chat.chat.title(), extra_info);
            }

            let mut chats_block = List::new(chat_titles).block(
                Block::default()
                    .title(app.chat_list.name)
                    .borders(Borders::ALL),
            );
            let mut chat_block = List::new(chat_history)
                .block(Block::default().title(chat_title).borders(Borders::ALL))
                .start_corner(Corner::BottomLeft);

            let mut input_block = Block::default()
                .title(app.input_box.name)
                .borders(Borders::ALL);
            let input = Paragraph::new(Text::styled(
                app.input_box.input_str.to_string(),
                unselected_style,
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(app.input_box.name),
            )
            .wrap(tui::widgets::Wrap { trim: true });

            match selected_block {
                TBlocks::ChatList => chats_block = chats_block.style(selected_style),
                TBlocks::CurrChat => chat_block = chat_block.style(selected_style),
                TBlocks::Input => input_block = input_block.style(selected_style),
            }

            f.render_widget(input_block, chunks[1]);
            f.render_widget(chats_block, chat_chunks[0]);
            f.render_widget(chat_block, chat_chunks[1]);
            f.render_widget(input, chunks[1]);
        })?;
        let enext = match events.next() {
            Ok(eve) => eve,
            Err(_e) => return Err(Error::new(ErrorKind::Other, "oh no!")),
        };
        if let Event::Input(input) = enext {
            match app.curr_mode {
                InputMode::Normal => match input {
                    Key::F(1) => {
                        tx.send(MsgCode::Exit).unwrap();
                        return Ok(());
                    }
                    Key::Char('\t') => {
                        selected_block = match selected_block {
                            TBlocks::ChatList => TBlocks::CurrChat,
                            TBlocks::CurrChat => TBlocks::Input,
                            TBlocks::Input => TBlocks::ChatList,
                        }
                    }
                    Key::Char('i') => app.curr_mode = InputMode::Insert,
                    _ => match selected_block {
                        TBlocks::ChatList => {
                            app.chat_list
                                .handle_input_normal(&app.outgoing_queue, &input);
                        }
                        TBlocks::CurrChat => {
                            app.chat_list
                                .chat_vec
                                .lock()
                                .unwrap()
                                .get_mut(*app.chat_list.selected_index.lock().unwrap() as usize)
                                .unwrap()
                                .handle_input_normal(&app.outgoing_queue, &input);
                        }
                        _ => {}
                    },
                },
                //TODO: get_cur_chat_function
                InputMode::Insert => {
                    let cur_chat_id = app
                        .chat_list
                        .chat_vec
                        .lock()
                        .unwrap()
                        .get((*app.chat_list.selected_index.lock().unwrap()) as usize)
                        .unwrap()
                        .chat
                        .id();
                    match input {
                        Key::Esc => app.curr_mode = InputMode::Normal,
                        _ => match selected_block {
                            TBlocks::ChatList => app.chat_list.handle_input_insert(
                                &app.outgoing_queue,
                                &input,
                                cur_chat_id,
                            ),
                            TBlocks::Input => app.input_box.handle_input_insert(
                                &app.outgoing_queue,
                                &input,
                                cur_chat_id,
                            ),

                            _ => {}
                        },
                    }
                }
            }
        }
    }
}
/*
 * Build the message list to be displayed, based on size parameters of chat box
 */

fn build_msg_list(
    chat: &TChat,
    ui_users: &HashMap<i64, TUser>,
    chat_box_width: usize,
    chat_box_height: usize,
    chat_history: &mut Vec<ListItem>,
) -> (usize, usize) {
    // Track total number of messages displayed, for tracking scroll

    let h = chat.history.lock().unwrap();

    let mut history_height = 0;
    let text_style = Style::default()
        .remove_modifier(Modifier::BOLD)
        .fg(Color::White);
    // Iterate through the chat hsitory, starting at the bottommost message that is to be displayed
    for msg in h[chat.bottom_index..].iter() {
        let msg_text = match msg.content().as_message_text() {
            Some(s) => s.text().text().to_string(),
            None => "[none]".to_string(),
        };
        let (sender_name, sender_color) = match ui_users.get(&msg.sender_user_id()) {
            Some(u) => (u.u.first_name().to_string(), u.color),
            None => ("Unknown User".to_string(), COLORS[0]),
        };
        let send_len = sender_name.chars().count();

        let full_msg = format!("{}: {}", sender_name, msg_text);
        let lines = textwrap::fill(&full_msg, chat_box_width);
        //let mut lis = Text::from("");
        let mut lis = Vec::new();
        for (i, l) in lines.lines().enumerate() {
            if l.len() == 0 {
                continue;
            }
            history_height += 1;
            if i == 0 {
                let mut first_line = vec![Span::styled(
                    sender_name.clone(),
                    Style::default()
                        .remove_modifier(Modifier::BOLD)
                        .fg(sender_color),
                )];
                let msg_slice: String = (*l).chars().skip(send_len).collect();
                first_line.push(Span::styled(msg_slice, text_style));
                lis.push(Spans::from(first_line));
                continue;
            }
            let t = Text::styled((*l).to_owned(), text_style).to_owned();
            lis.extend(t.lines);
        }

        // Peel off lines from the start of the topmost message to display partial message
        // when cut off
        while history_height > chat_box_height {
            lis.remove(0);
            history_height -= 1;
        }
        let t = Text::from(lis);
        chat_history.push(ListItem::new(t));

        if history_height >= chat_box_height {
            break;
        }
    }
    return (chat_history.len(), history_height);
}
/*
 *   Get specified command line argument
 */

fn get_arg(arg_name: &str) -> Option<String> {
    for arg in std::env::args() {
        if &arg[..arg_name.len()] == arg_name {
            return Some(arg.split("=").collect::<Vec<&str>>()[1].trim().to_string());
        }
    }
    None
}
fn parse_msg(cur_msg: &mut Value, chat_id: i64) -> Message {
    // ANOTHER WEIRD STOPGAP
    cur_msg["sender_user_id"] = cur_msg["sender"]["user_id"].to_owned();
    cur_msg["views"] = json!(1);
    let cur_msg = match Message::from_json(cur_msg.to_string()) {
        Err(_e) => {
            let msg_builder = &mut Message::builder();
            let formatted_str = match cur_msg["content"]["@type"].as_str().unwrap() {
                "messageSticker" => {
                    format!("[{} Sticker]", cur_msg["content"]["sticker"]["emoji"])
                }
                "messageText" => match cur_msg["content"].get("web_page") {
                    Some(_c) => {
                        let wp = &cur_msg["content"]["web_page"];
                        format!(
                            "{}\n{}\n{}\n{}",
                            cur_msg["content"]["text"]["text"].as_str().unwrap(),
                            wp["site_name"].as_str().unwrap(),
                            wp["title"].as_str().unwrap(),
                            wp["description"]["text"].as_str().unwrap()
                        )
                    }

                    None => "[none]".to_owned(),
                },
                t => {
                    eprintln!("truly the type is {}", t);
                    "[none]".to_owned()
                }
            };
            msg_builder
                .content(MessageContent::MessageText(
                    MessageText::builder()
                        .text(FormattedText::builder().text(formatted_str))
                        .build(),
                ))
                .chat_id(chat_id)
                .id(cur_msg["id"].as_i64().unwrap())
                .date(cur_msg["date"].as_i64().unwrap())
                .sender_user_id(cur_msg["sender_user_id"].as_i64().unwrap())
                .build()
        }
        Ok(ok) => ok,
    };
    return cur_msg;
}
