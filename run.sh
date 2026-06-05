#!/bin/bash

# Kill any existing processes
echo "🧹 Cleaning up existing processes..."
fuser -k 3000/tcp 2>/dev/null
fuser -k 6800/tcp 2>/dev/null # Aria2 port
pkill -f backend 2>/dev/null
pkill -f aria2c 2>/dev/null

# Kill background processes on exit
trap "kill 0" EXIT

echo "🚀 Starting Download Manager..."

# Start Aria2 in RPC mode for Torrents/Magnets
echo "📡 Starting Aria2 Engine..."
aria2c --enable-rpc --rpc-listen-all=false --rpc-allow-origin-all --daemon=false --quiet=true &
ARIA_PID=$!

# Start backend
echo "📦 Starting Rust backend..."
cd backend
cargo run & 
BACKEND_PID=$!

# Wait for backend to be ready
sleep 2

# Start frontend
echo "💻 Starting Electron frontend..."
cd ../frontend
npm start -- --no-sandbox &
FRONTEND_PID=$!

# Wait for processes
wait $BACKEND_PID $FRONTEND_PID $ARIA_PID
