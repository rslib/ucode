//! Interactive demo: launch the TUI with a fake echo "LLM".
//!
//! Run with: cargo run -p ucode-tui --example demo
//!
//! Type a message and press Enter. The demo echoes your message back
//! as a streaming response with a typewriter effect. Ctrl+C to exit.

use std::time::Duration;

use tokio::sync::mpsc;
use ucode_tui::event_loop::TuiEvent;

/// Simulated LLM that echoes the user's message back as streaming tokens.
async fn fake_llm(
    mut user_rx: mpsc::UnboundedReceiver<String>,
    tui_tx: mpsc::UnboundedSender<TuiEvent>,
) {
    // Send a welcome message on startup.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = tui_tx.send(TuiEvent::SystemMessage(
        "Demo mode -- type a message and press Enter".to_owned(),
    ));

    while let Some(user_msg) = user_rx.recv().await {
        // Simulate a brief "thinking" delay.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Build a response from the user's message.
        let response = format!(
            "You said: \"{user_msg}\"\n\n\
             This is a demo echo response. In the real app, this would be \
             streamed from an LLM provider.\n\n\
             Try typing another message!"
        );

        // Stream the response word-by-word for a natural typewriter effect.
        let words: Vec<&str> = response
            .split_inclusive(|c: char| c.is_whitespace())
            .collect();
        for word in words {
            if tui_tx.send(TuiEvent::StreamToken(word.to_owned())).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(35)).await;
        }

        let _ = tui_tx.send(TuiEvent::StreamDone);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tui_tx, tui_rx) = ucode_tui::create_event_channel();
    let (user_tx, user_rx) = mpsc::unbounded_channel::<String>();

    // Spawn the fake LLM responder.
    tokio::spawn(fake_llm(user_rx, tui_tx));

    // Run the TUI, passing the user message channel so SendMessage
    // notifications reach our fake LLM.
    ucode_tui::run(tui_rx, Some(user_tx)).await
}
