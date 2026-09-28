import { Button, Code, Stack, Text } from "@chakra-ui/react";
import { useEffect, useState } from "react";
import { Frame } from "./components";
import { getCurrentWindow, invoke } from "./tauri";
export function CctkWarningApp() { const [message, setMessage] = useState("Loading details..."); useEffect(() => { invoke("cmd_get_cctk_warning").then(value => setMessage(value || "No diagnostic output was returned.")).catch(error => setMessage(String(error))); }, []); return <Frame title="Dell Command Configure Warning"><Stack p="5" spacing="4"><Text fontSize="lg" fontWeight="bold">Mode Change Failed</Text><Text color="gray.600">Dell Command | Configure returned:</Text><Code p="3" whiteSpace="pre-wrap" maxH="120px" overflow="auto">{message}</Code><Button alignSelf="end" onClick={() => getCurrentWindow().close()}>OK</Button></Stack></Frame>; }
