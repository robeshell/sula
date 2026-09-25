// Dev-only entry for /mock.html: install the mock backend, then boot the real app.
import { installMockBackend } from "./mockBackend";

installMockBackend();
void import("../main");
