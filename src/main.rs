use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::vec;

use crossbeam::thread;
/*
use termion::raw::IntoRawMode;
use tui::backend::TermionBackend;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::text::Span;
use tui::widgets::{Block, Borders};
use tui::Terminal;
*/

use rtdlib::types::*;
use rtdlib::Tdlib;

const TIMEOUT: f64 = 60.0;

fn main() {
    let (api_id, api_hash, phone_number) = read_info().unwrap();
    let tdlib = Tdlib::new();
    let mut chat_vec = vec::Vec::new();
    //Main listening loop
    thread::scope(|s| {
        let rec_thread = s.spawn(|_| {
            loop {
                match tdlib.receive(TIMEOUT) {
                    Some(res) => {
                        let obj = json::parse(&res).unwrap();
                        match obj["@type"].as_str().unwrap() {
                            "updateAuthorizationState" => {
                                let astate = &obj["authorization_state"];
                                println!("astate {}", &astate);
                                match astate["@type"].as_str().unwrap() {
                                    "authorizationStateReady" => {
                                        //TODO: store auth credentials
                                        setup_interface(&tdlib);
                                    }
                                    "authorizationStateWaitTdlibParameters" => {
                                        send_tdlib_parameters(&tdlib, api_id, &api_hash);
                                    }
                                    "authorizationStateWaitEncryptionKey" => {
                                        send_check_encryption_key(&tdlib)
                                    }
                                    "authorizationStateWaitPhoneNumber" => {
                                        send_phone_parameters(&tdlib, &phone_number);
                                    }
                                    _ => {
                                        //TODO: verify more formally
                                        if astate.has_key("code_info")
                                            && astate["code_info"].has_key("type")
                                            && astate["code_info"]["type"]["@type"]
                                                == "authenticationCodeTypeTelegramMessage"
                                        {
                                            println!("Input that code king!");
                                            let mut input_text = String::new();
                                            io::stdin()
                                                .read_line(&mut input_text)
                                                .expect("failed to read from stdin");

                                            let check_auth_code =
                                                CheckAuthenticationCode::builder()
                                                    .code(input_text)
                                                    .build();

                                            tdlib.send(&check_auth_code.to_json().unwrap());
                                        }
                                    }
                                }
                            } // end updateAuthorizationState

                            "updateNewChat" => {
                                let new_chat = &mut obj["chat"].clone();

                                // BEGIN WEIRD STOPGAP I SHOULD PROBABLY RESOLVE
                                new_chat["order"] = json::JsonValue::from("1");
                                new_chat["is_pinned"] = json::JsonValue::from(false);
                                new_chat["is_sponsored"] = json::JsonValue::from(false);
                                new_chat["pinned_message_id"] = json::JsonValue::from(0);
                                //END WEIRD STOPGAP

                                chat_vec.push(Chat::from_json(new_chat.to_string()));
                            }
                            _ => {
                                println!("Res: {}, {}", res, obj["@type"]);
                            }
                        }
                    }
                    None => {
                        println!("There was an error!");
                    }
                }
            }
        });

        let _rec_result = rec_thread.join().unwrap();
    })
    .unwrap();
    /*
    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear();
    terminal.draw(|f| {
        let size = f.size();
        let block = Block::default().title("Block").borders(Borders::ALL);
        f.render_widget(block, size);

        let myname = Block::default().title(Span::styled(
            "Joseph Barr",
            Style::default().fg(Color::Yellow),
        ));

        let name_area = Rect::new(5, 5, 10, 10);
        f.render_widget(myname, name_area);
    })
    */
}

fn read_info() -> io::Result<(i64, String, String)> {
    let file = File::open("info.txt")?;
    let mut reader = BufReader::new(file);

    let mut api_id = String::new();
    let mut api_hash = String::new();
    let mut phone_number = String::new();

    reader.read_line(&mut api_id)?;
    reader.read_line(&mut api_hash)?;
    reader.read_line(&mut phone_number)?;

    let api_id: i64 = api_id.trim().parse().unwrap();

    Ok((
        api_id,
        api_hash.trim().to_string(),
        phone_number.trim().to_string(),
    ))
}

fn setup_interface(tdlib: &Tdlib) {
    let chat_list = ChatList::default();
    let chat_list_req = GetChats::builder()
        .chat_list(chat_list)
        .offset_order(0)
        .offset_chat_id(0)
        .limit(255)
        .build();
    tdlib.send(&chat_list_req.to_json().unwrap());
}

fn send_tdlib_parameters(tdlib: &Tdlib, api_id: i64, api_hash: &str) {
    let set_tdlib_parameters = SetTdlibParameters::builder()
        .parameters(
            TdlibParameters::builder()
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

fn send_check_encryption_key(tdlib: &Tdlib) {
    let check_enc_key = CheckDatabaseEncryptionKey::builder().build();
    tdlib.send(&check_enc_key.to_json().unwrap());
}

fn send_phone_parameters(tdlib: &Tdlib, phone_number: &str) {
    let phone_parameters = SetAuthenticationPhoneNumber::builder()
        .phone_number(phone_number)
        .settings(PhoneNumberAuthenticationSettings::builder().build())
        .build();

    tdlib.send(&phone_parameters.to_json().unwrap());
}
