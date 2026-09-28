// Hands the page, link or selection to RQuickShare's native host, which opens
// the app's send screen to pick a nearby device.
const HOST = "dev.mandre.rquickshare"

function send(text) {
  if (!text) return
  chrome.runtime.sendNativeMessage(HOST, { text }, (reply) => {
    const error = chrome.runtime.lastError?.message ?? reply?.error
    if (error) {
      chrome.action.setBadgeBackgroundColor({ color: "#d93025" })
      chrome.action.setBadgeText({ text: "!" })
      chrome.action.setTitle({ title: `RQuickShare: ${error}` })
      setTimeout(() => chrome.action.setBadgeText({ text: "" }), 4000)
    }
  })
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "page",
    title: "Send page with RQuickShare",
    contexts: ["page"],
  })
  chrome.contextMenus.create({
    id: "link",
    title: "Send link with RQuickShare",
    contexts: ["link"],
  })
  chrome.contextMenus.create({
    id: "image",
    title: "Send image link with RQuickShare",
    contexts: ["image"],
  })
  chrome.contextMenus.create({
    id: "selection",
    title: "Send “%s” with RQuickShare",
    contexts: ["selection"],
  })
})

chrome.contextMenus.onClicked.addListener((info, tab) => {
  switch (info.menuItemId) {
    case "page":
      return send(info.pageUrl ?? tab?.url)
    case "link":
      return send(info.linkUrl)
    case "image":
      return send(info.srcUrl)
    case "selection":
      return send(info.selectionText)
  }
})

chrome.action.onClicked.addListener((tab) => send(tab.url))
