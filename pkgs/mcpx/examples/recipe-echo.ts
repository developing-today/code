// Echo a message a few times through the demo server.
//
// The header declares the holes; `mcpx recipes run` fills them. A plain
// `mcpx run` refuses this file, because @message would reach the runtime
// unfilled.
//
// @param message:string   what to echo
// @param times:number = 2   how many times

for (let i = 0; i < @times; i++) {
  console.log(String(await tools.demo.echo({ message: @message })));
}
