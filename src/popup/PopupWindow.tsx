import { readKind } from "./popupShared";
import { SessionPopup } from "./SessionPopup";
import { UpdatePopup } from "./UpdatePopup";

export default function PopupWindow() {
  const kind = readKind();
  if (kind === "update") {
    return <UpdatePopup />;
  }
  return <SessionPopup />;
}
