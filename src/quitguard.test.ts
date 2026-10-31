import { beforeEach, describe, expect, it } from "vitest";
import { askQuit, quitText } from "./quitguard";

const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("#quit-host button")];
const press = (label: string) => buttons().find((b) => b.textContent === label)!.click();

describe("quitText", () => {
  it("names the sessions that will stop", () => {
    expect(quitText(["fix-ci", "docs"], false)).toEqual({
      text: "These Native sessions are working and will stop if Maya quits: fix-ci, docs.",
      yes: "Quit anyway",
    });
    expect(quitText(["fix-ci"], true)).toEqual({
      text: "This Native session is working and will stop when Maya restarts to update: fix-ci.",
      yes: "Restart anyway",
    });
  });
});

describe("askQuit", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"><div class="modal">viewer</div></div>';
  });

  it("resolves true on the yes button and false on Cancel", async () => {
    const yes = askQuit(["fix-ci"], false);
    expect(document.querySelector("#quit-host p")?.textContent).toBe("This Native session is working and will stop if Maya quits: fix-ci.");
    expect(buttons().map((b) => b.textContent)).toEqual(["Cancel", "Quit anyway"]);
    press("Quit anyway");
    await expect(yes).resolves.toBe(true);
    expect(buttons()).toEqual([]);

    const no = askQuit(["fix-ci"], true);
    press("Cancel");
    await expect(no).resolves.toBe(false);
    expect(buttons()).toEqual([]);
  });

  it("leaves an open session viewer where it is, and survives its repaint", async () => {
    const answer = askQuit(["fix-ci"], false);
    expect(document.querySelector("#modal-host")?.textContent).toBe("viewer");
    document.getElementById("modal-host")!.replaceChildren();
    press("Cancel");
    await expect(answer).resolves.toBe(false);
  });

  it("Escape cancels without reaching the viewer's own Escape", async () => {
    let reached = false;
    document.addEventListener("keydown", () => (reached = true), { once: true });
    const answer = askQuit(["fix-ci"], false);
    document.querySelector("#quit-host .modal__panel")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await expect(answer).resolves.toBe(false);
    expect(reached).toBe(false);
  });

  it("a second ask replaces the first, which counts as Cancel", async () => {
    const first = askQuit(["fix-ci"], false);
    const second = askQuit(["fix-ci", "docs"], false);
    await expect(first).resolves.toBe(false);
    expect(document.querySelectorAll("#quit-host .modal").length).toBe(1);
    press("Quit anyway");
    await expect(second).resolves.toBe(true);
  });
});
