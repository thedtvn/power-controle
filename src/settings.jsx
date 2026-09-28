import { Alert, AlertIcon, Box, Button, Code, Flex, FormControl, FormLabel, Grid, Input, Select, Stack, Switch, Text, useColorMode, useToast } from "@chakra-ui/react";
import { useEffect, useState } from "react";
import { GridLoader } from "react-spinners";
import { Frame, Section, Status, SaveButton } from "./components";
import { theme } from "./theme";
import { invoke, isTauriRuntime, listen } from "./tauri";

const thermalLabels = { optimized: "Optimized", cool: "Cool", quiet: "Quiet", ultra_performance: "Ultra Performance" };
const batteryLabels = { standard: "Standard", express: "ExpressCharge", primarily_ac: "Primarily AC Use", adaptive: "Adaptive", custom: "Custom" };
const powerLabels = { balanced: "Balanced", high_performance: "High Performance", power_saver: "Power Saver" };
const baseSetting = (mode = "adaptive") => ({ mode, custom_start: 50, custom_stop: 85 });

export function SettingsApp() {
  const toast = useToast();
  const { colorMode, setColorMode } = useColorMode();
  const [status, setStatus] = useState(null);
  const [overview, setOverview] = useState(null);
  const [dell, setDell] = useState(null);
  const [config, setConfig] = useState(null);
  const [saving, setSaving] = useState(false);
  const [autostart, setAutostart] = useState(false);
  const [autostartSaving, setAutostartSaving] = useState(false);
  const [loadError, setLoadError] = useState("");

  const refresh = async () => {
    const [s, o, d, c, startup] = await Promise.all([invoke("cmd_get_status"), invoke("cmd_get_battery_overview"), invoke("cmd_get_dell_state"), invoke("cmd_get_config"), invoke("cmd_get_autostart")]);
    setStatus(s); setOverview(o); setDell(d); setConfig(c); setAutostart(startup);
  };
  useEffect(() => { refresh().catch(error => setLoadError(String(error))); const unlisten = Promise.all([listen("status-changed", e => setStatus(e.payload)), listen("battery-overview-changed", e => setOverview(e.payload)), listen("thermal-changed", e => setDell(e.payload)), listen("battery-setting-changed", e => setDell(e.payload)), listen("cctk-action-failed", e => toast({ status: "error", title: "Mode change failed", description: e.payload }))]); return () => { unlisten.then(values => values.forEach(stop => stop())).catch(() => {}); }; }, []);
  if (!isTauriRuntime) return <Frame settings><Stack p="6" spacing="3"><Text fontSize="lg" fontWeight="bold">Power Control requires Tauri</Text><Text>Vite is serving React correctly, but the Windows power APIs are available only inside the Tauri desktop process.</Text><Code>cargo tauri dev</Code></Stack></Frame>;
  if (loadError) return <Frame settings><Alert status="error" m="5"><AlertIcon />{loadError}</Alert></Frame>;
  if (!config) return <Frame settings><Flex h="calc(100% - 32px)" align="center" justify="center" direction="column" gap="4"><GridLoader color="#0067c0" size={12} /><Text color="gray.500">Loading settings…</Text></Flex></Frame>;
  const updatePower = (key, value) => setConfig(c => ({ ...c, power: { ...c.power, [key]: value } }));
  const thermal = config.thermal || { auto_switch: false, on_ac: "optimized", on_battery: "quiet" };
  const battery = config.battery || { auto_switch: false, on_ac: baseSetting("primarily_ac"), on_battery: baseSetting() };
  const updateThermal = (key, value) => setConfig(c => ({ ...c, thermal: { ...thermal, [key]: value } }));
  const updateBattery = (path, value) => setConfig(c => ({ ...c, battery: { ...battery, [path]: value } }));
  const save = async () => { setSaving(true); try { await invoke("cmd_save_config", { config }); toast({ status: "success", title: "Settings saved" }); } catch (error) { toast({ status: "error", title: "Settings could not apply", description: String(error) }); } finally { setSaving(false); } };
  const changeAutostart = async (enabled) => { setAutostartSaving(true); try { await invoke("cmd_set_autostart", { enabled }); setAutostart(enabled); toast({ status: "success", title: enabled ? "Start with Windows enabled" : "Start with Windows disabled" }); } catch (error) { toast({ status: "error", title: "Could not change startup setting", description: String(error) }); } finally { setAutostartSaving(false); } };
  const install = async () => { try { await invoke("cmd_install_cctk"); const refreshed = await invoke("cmd_refresh_dell_state"); setDell(refreshed); toast({ status: "success", title: "Dell Command Configure installed", description: "Restart may be recommended." }); } catch (error) { toast({ status: "error", title: "Installation failed", description: String(error) }); } };
  const modes = dell?.supported_battery_modes || [];
  return <Frame settings><Stack p={{ base: 3, md: 5 }} spacing="4" overflowY="auto" h="calc(100% - 32px)">
    <Section title="Current Status"><Grid templateColumns={{ base: "1fr", sm: "repeat(3, 1fr)" }} gap="3"><Status label="Power plan" value={powerLabels[status?.active_scheme]} /><Status label="Power source" value={status?.power_source === "ac" ? "AC Power" : status?.power_source} /><Status label="Thermal" value={thermalLabels[dell?.current_profile] || "—"} /></Grid></Section>
    <Section title="Battery Overview">{overview ? (overview.available ? <Grid templateColumns={{ base: "1fr", sm: "repeat(2, 1fr)" }} gap="3"><Status label="State" value={overview.state?.replaceAll("_", " ")} /><Status label="Rate" value={overview.chargeRateMw ? `${overview.chargeRateMw / 1000} W` : "—"} /><Status label="Health" value={overview.healthPercent ? `${overview.healthPercent}%` : "—"} /><Status label="Cycles" value={overview.cycleCount} /><Status label="Model" value={overview.model} /><Status label="Voltage" value={overview.voltageMv ? `${overview.voltageMv} mV` : "—"} /></Grid> : <Alert status="info"><AlertIcon />Battery information is unavailable.</Alert>) : <Flex minH="96px" align="center" justify="center" direction="column" gap="3"><GridLoader color="#0067c0" size={10} /><Text color="gray.500" fontSize="sm">Loading battery information…</Text></Flex>}</Section>
    <Section title="General"><Stack spacing="4"><Switch isChecked={autostart} isDisabled={autostartSaving} onChange={e => changeAutostart(e.target.checked)}>Start with Windows</Switch><FormControl><FormLabel>Appearance</FormLabel><Select value={colorMode} onChange={e => setColorMode(e.target.value)}><option value="light">Light</option><option value="dark">Dark</option></Select></FormControl></Stack></Section>
    <Section title="Power Auto-Switch"><Stack spacing="3"><Switch isChecked={config.power.auto_switch} onChange={e => updatePower("auto_switch", e.target.checked)}>Enable automatic switching</Switch><Choice label="On AC" value={config.power.on_ac} choices={powerLabels} onChange={v => updatePower("on_ac", v)} /><Choice label="On Battery" value={config.power.on_battery} choices={powerLabels} onChange={v => updatePower("on_battery", v)} /></Stack></Section>
    {!dell?.available ? <Section title="Dell Controls"><Alert status="info" mb="3"><AlertIcon />Dell Command | Configure is required.</Alert><Button onClick={install}>Install Dell Command | Configure</Button></Section> : <>
      <Section title="Thermal Management"><Stack spacing="3"><Switch isChecked={thermal.auto_switch} onChange={e => updateThermal("auto_switch", e.target.checked)}>Enable thermal auto-switch</Switch><Choice label="On AC" value={thermal.on_ac} choices={thermalLabels} allowed={dell.supported_profiles} onChange={v => updateThermal("on_ac", v)} /><Choice label="On Battery" value={thermal.on_battery} choices={thermalLabels} allowed={dell.supported_profiles} onChange={v => updateThermal("on_battery", v)} /></Stack></Section>
      <Section title="Battery Management"><Stack spacing="3"><Switch isChecked={battery.auto_switch} onChange={e => updateBattery("auto_switch", e.target.checked)}>Enable battery auto-switch</Switch><BatteryChoice label="On AC" value={battery.on_ac} modes={modes} onChange={v => updateBattery("on_ac", v)} /><BatteryChoice label="On Battery" value={battery.on_battery} modes={modes} onChange={v => updateBattery("on_battery", v)} /></Stack></Section>
    </>}
    <Flex position="sticky" bottom="0" zIndex="1" justify="end" py="3" bg="inherit"><SaveButton loading={saving} onClick={save} /></Flex>
  </Stack></Frame>;
}
function Choice({ label, value, choices, allowed, onChange }) { const entries = Object.entries(choices).filter(([key]) => !allowed || allowed.includes(key)); return <FormControl><FormLabel>{label}</FormLabel><Select value={value} onChange={e => onChange(e.target.value)}>{entries.map(([key, text]) => <option key={key} value={key}>{text}</option>)}</Select></FormControl>; }
function BatteryChoice({ label, value, modes, onChange }) { const mode = value.mode; return <Stack spacing="2"><Choice label={label} value={mode} choices={batteryLabels} allowed={modes} onChange={next => onChange({ ...value, mode: next })} />{mode === "custom" && <Grid templateColumns="repeat(2, 1fr)" gap="3"><FormControl><FormLabel>Start %</FormLabel><Input type="number" min="50" max="95" value={value.custom_start} onChange={e => onChange({ ...value, custom_start: Number(e.target.value) })} /></FormControl><FormControl><FormLabel>Stop %</FormLabel><Input type="number" min="55" max="100" value={value.custom_stop} onChange={e => onChange({ ...value, custom_stop: Number(e.target.value) })} /></FormControl></Grid>}</Stack>; }
