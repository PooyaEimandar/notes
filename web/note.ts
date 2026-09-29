// The share buttons on a note. They use the device's share sheet where there is
// one, and copy the link otherwise.

export {};

const COPIED_MS = 2000;

for (const button of document.querySelectorAll<HTMLButtonElement>("[data-share]")) {
  const label = button.textContent ?? "Share";
  button.hidden = false;
  button.addEventListener("click", async () => {
    const title = button.dataset.title ?? document.title;
    const url = button.dataset.share || location.href;
    try {
      if (navigator.share) {
        await navigator.share({ title, url });
        return;
      }
      await navigator.clipboard.writeText(url);
      button.textContent = "Link copied";
    } catch (error) {
      // Closing the share sheet is not a failure.
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      button.textContent = "Copy failed";
    }
    window.setTimeout(() => {
      button.textContent = label;
    }, COPIED_MS);
  });
}
