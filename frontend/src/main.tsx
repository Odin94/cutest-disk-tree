import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { getDebugLogPath, debugLog, debugLogStats } from "./api";

getDebugLogPath()
  .then((path) => {
    debugLog(`app started debug_log_path=${path}`);
    debugLogStats("start");
  })
  .catch(() => {});

const DEBUG_STATS_INTERVAL_MS = 5000;
setInterval(() => {
  debugLogStats("tick");
}, DEBUG_STATS_INTERVAL_MS);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
);
