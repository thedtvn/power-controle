import { ChakraProvider } from "@chakra-ui/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BiosPasswordApp } from "./bios-password";
import { CctkWarningApp } from "./cctk-warning";
import { SettingsApp } from "./settings";
import { theme } from "./theme";

const view = new URLSearchParams(window.location.search).get("view");
const Screen = view === "bios-password"
  ? BiosPasswordApp
  : view === "cctk-warning"
    ? CctkWarningApp
    : SettingsApp;

createRoot(document.getElementById("root")).render(
  <StrictMode>
    <ChakraProvider theme={theme}>
      <Screen />
    </ChakraProvider>
  </StrictMode>,
);
