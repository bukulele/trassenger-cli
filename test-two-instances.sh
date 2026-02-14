#!/bin/bash
# Test script to run two separate Trassenger instances

echo "🚀 Trassenger TUI - Two Instance Testing"
echo ""
echo "This will open two terminal windows:"
echo "  - Alice (left): TRASSENGER_DATA_DIR=~/.trassenger-alice"
echo "  - Bob (right): TRASSENGER_DATA_DIR=~/.trassenger-bob"
echo ""
echo "Steps to test:"
echo "  1. Wait for both to start"
echo "  2. In Alice: /export → enter 'Alice' → check Downloads for contact-Alice.json"
echo "  3. In Bob: /export → enter 'Bob' → check Downloads for contact-Bob.json"
echo "  4. In Alice: /import → drag contact-Bob.json → Enter"
echo "  5. In Bob: /import → drag contact-Alice.json → Enter"
echo "  6. In Alice: type 'Hello Bob!' → Enter"
echo "  7. Wait ~5-10s for Bob to receive (watch polling)"
echo "  8. In Bob: see 'Hello Bob!' appear, reply 'Hi Alice!'"
echo ""
read -p "Press Enter to launch both instances..."

# Build first
cargo build --release

# Open Alice in new terminal
osascript -e 'tell application "Terminal" to do script "cd '"$(pwd)"' && TRASSENGER_DATA_DIR=~/.trassenger-alice ./target/release/trassenger-tui"'

# Wait a moment
sleep 1

# Open Bob in new terminal
osascript -e 'tell application "Terminal" to do script "cd '"$(pwd)"' && TRASSENGER_DATA_DIR=~/.trassenger-bob ./target/release/trassenger-tui"'

echo ""
echo "✓ Both instances launched in separate Terminal windows!"
echo "  Alice: ~/.trassenger-alice/"
echo "  Bob: ~/.trassenger-bob/"
