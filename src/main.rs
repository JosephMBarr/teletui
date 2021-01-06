use std::fs::File;
use std::io::{self, BufRead, BufReader};
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

const TIMEOUT: f64 = 5.0;

fn main() {
    let (api_id, api_hash, phone_number) = read_info().unwrap();

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
    let phone_parameters = SetAuthenticationPhoneNumber::builder()
        .phone_number(phone_number)
        .settings(PhoneNumberAuthenticationSettings::builder().build())
        .build();
    let tdlib = Tdlib::new();

    match tdlib.receive(TIMEOUT) {
        Some(res) => println!("Res: {}", res),
        None => return,
    }
    match tdlib.receive(TIMEOUT) {
        Some(res) => println!("Res: {}", res),
        None => return,
    }
    tdlib.send(&set_tdlib_parameters.to_json().unwrap());
    match tdlib.receive(TIMEOUT) {
        Some(res) => println!("Res: {}", res),
        None => return,
    }

    let check_enc_key = CheckDatabaseEncryptionKey::builder().build();
    tdlib.send(&check_enc_key.to_json().unwrap());
    match tdlib.receive(TIMEOUT) {
        Some(res) => println!("Res: {}", res),
        None => return,
    }

    tdlib.send(&phone_parameters.to_json().unwrap());
    loop {
        match tdlib.receive(60.0) {
            Some(res) => {
                let obj = json::parse(&res).unwrap();
                println!("Res: {}, {}", res, obj["@type"]);

                if obj["@type"] == "updateAuthorizationState"
                    && obj["authorization_state"].has_key("code_info")
                    && obj["authorization_state"]["code_info"].has_key("type")
                    && obj["authorization_state"]["code_info"]["type"]["@type"]
                        == "authenticationCodeTypeTelegramMessage"
                {
                    println!("Input that code king!");
                    let mut input_text = String::new();
                    io::stdin()
                        .read_line(&mut input_text)
                        .expect("failed to read from stdin");

                    let check_auth_code =
                        CheckAuthenticationCode::builder().code(input_text).build();

                    tdlib.send(&check_auth_code.to_json().unwrap());
                    let me_request = r#"{"@type": "getUser"}"#;
                    tdlib.send(me_request);
                }
            }
            None => return,
        }
    }
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
