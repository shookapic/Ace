import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { FilePreview } from "./routes/FilePreview";
import "./styles/globals.css";

// The dedicated file-preview window loads the same bundle; render the viewer
// there instead of the full chat app.
const isPreview = (() => {
  try {
    return getCurrentWindow().label === "file-preview";
  } catch {
    return false;
  }
})();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isPreview ? <FilePreview /> : <App />}</React.StrictMode>,
);
