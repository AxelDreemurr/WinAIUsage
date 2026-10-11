import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

async function bootstrap() {
  // Running in a plain browser (no Tauri runtime): use dummy data.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installDevMock } = await import("./devMock");
    installDevMock();
  }

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

bootstrap();
