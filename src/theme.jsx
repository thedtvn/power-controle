import { extendTheme } from "@chakra-ui/react";

export const theme = extendTheme({
  config: {
    initialColorMode: "system",
    useSystemColorMode: true,
  },
  styles: {
    global: {
      "html, body, #root": { height: "100%" },
      body: { bg: "chakra-body-bg", color: "chakra-body-text" },
    },
  },
  colors: { brand: { 500: "#0067c0", 600: "#005a9e" } },
  components: {
    Button: { defaultProps: { colorScheme: "brand" } },
  },
});
