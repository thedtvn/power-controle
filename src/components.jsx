import { Box, Button, Flex, IconButton, Text, useColorModeValue } from "@chakra-ui/react";
import { getCurrentWindow, invoke } from "./tauri";

export function Frame({ children, settings = false, title = "Power Control" }) {
  const background = useColorModeValue("gray.100", "gray.900");
  const foreground = useColorModeValue("gray.900", "gray.100");
  const titlebar = useColorModeValue("gray.200", "gray.800");
  const closeColor = useColorModeValue("gray.700", "gray.200");
  const close = () => settings
    ? invoke("cmd_close_settings")
    : getCurrentWindow().close();
  return <Box h="100%" bg={background} color={foreground}>
    <Flex h="32px" px="3" align="center" bg={titlebar} borderBottomWidth="1px" data-tauri-drag-region>
      <Text flex="1" fontSize="sm" fontWeight="medium" data-tauri-drag-region>{title}</Text>
      <IconButton aria-label="Close" size="xs" variant="ghost" color={closeColor} _hover={{ bg: "red.500", color: "white" }} onClick={close} icon={<Box as="svg" width="10px" height="10px" viewBox="0 0 10 10"><path d="M1 1L9 9M9 1L1 9" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></Box>} />
    </Flex>
    {children}
  </Box>;
}

export function Status({ label, value }) {
  return <Box><Text fontSize="xs" color="gray.500" textTransform="uppercase">{label}</Text><Text fontWeight="semibold">{value ?? "—"}</Text></Box>;
}

export function Section({ title, children }) {
  const background = useColorModeValue("white", "gray.800");
  const heading = useColorModeValue("gray.600", "gray.300");
  return <Box bg={background} borderWidth="1px" borderRadius="md" p="4"><Text fontSize="sm" fontWeight="bold" color={heading} textTransform="uppercase" mb="3">{title}</Text>{children}</Box>;
}

export function SaveButton({ loading, onClick }) {
  return <Button alignSelf="end" bg="brand.500" color="white" _hover={{ bg: "brand.600" }} _disabled={{ bg: "brand.500", color: "white", opacity: 0.55, cursor: "not-allowed" }} isLoading={loading} onClick={onClick}>Save & Apply Settings</Button>;
}
