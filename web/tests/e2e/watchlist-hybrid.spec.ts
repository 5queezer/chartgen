// A laptop with both a mouse and a touch screen: the mouse is the primary pointer,
// so it reports `hover: hover`, yet a tap can land on a row. Its own file because
// launch options force a new worker. hasTouch would report `hover: none`, a
// different device, so the media values are set directly.

import { test, expect } from "./fixtures/app";

test.use({
  launchOptions: {
    args: [
      "--blink-settings=primaryHoverType=2,primaryPointerType=4,availableHoverTypes=3,availablePointerTypes=6",
    ],
  },
});

test("watchlist row buttons show always, so a tap never lands on a hidden one", async ({
  bootedApp: page,
}) => {
  const media = await page.evaluate(() => ({
    hover: matchMedia("(hover: hover)").matches,
    coarse: matchMedia("(any-pointer: coarse)").matches,
  }));
  expect(media).toEqual({ hover: true, coarse: true });
  await page.mouse.move(0, 0);
  const button = (name: string) => page.getByRole("button", { name, exact: true });
  await expect(button("Remove BTCUSDT from watchlist")).toHaveCSS("opacity", "1");
  await expect(button("Flag BTCUSDT")).toHaveCSS("opacity", "1");
});
