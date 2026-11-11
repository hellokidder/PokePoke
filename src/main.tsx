import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { bootstrapDocument } from "./lib/theme";
import "./index.css";

bootstrapDocument();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
