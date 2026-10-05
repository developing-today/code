await tools.mcp_chrome_devtools.navigate_page({ url: "https://manus.im/pricing" });
await tools.mcp_chrome_devtools.wait_for({ text: "month", timeout: 20000 }).catch(e=>console.log("waitfail",e.message));
const snap = await tools.mcp_chrome_devtools.take_snapshot({});
console.log(typeof snap === "string" ? snap : JSON.stringify(snap));
