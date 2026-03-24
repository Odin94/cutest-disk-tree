import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { getDebugLogPath, debugLog, debugLogStats } from "./api";

const t0 = performance.now();

getDebugLogPath()
  .then((path) => {
    debugLog(`app started debug_log_path=${path}`);
    debugLogStats("start");
  })
  .catch(() => { });

const DEBUG_STATS_INTERVAL_MS = 15000;
setInterval(() => {
  debugLogStats("tick");
}, DEBUG_STATS_INTERVAL_MS);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
);

// Two rAF frames = after browser has painted the first frame.
requestAnimationFrame(() => {
  requestAnimationFrame(() => {
    debugLog(`perf: time_to_first_render_ms=${Math.round(performance.now() - t0)}`);
  });
});
