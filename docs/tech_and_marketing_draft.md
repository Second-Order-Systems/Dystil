Dystil is a local app that captures the work user does continiously via AXTrees and UIAutomations and then cleans it and stores it locally. Nothing ever leaves the device.

The stored data is then exposed via MCP for external AI agents so that user can plug it into their claude code or codex and ask it questions.

Examples:
 "What did i work on toady?"
 "What was name of doc where i noted down our leads?"
 "Can you give me notes for todays standup?"
 "I was facing a persistant bug yesterday, can you look what that bug was, replicate it and the suggest a fix."

Dystil also does two passes of PII redaction to maintain higest level of privacy locally possible.

Dystil can also look at users activity and repeated work and suggest automation opportunites to help them free up hours.

# How capture works

Dystil doenst always caputre data to avoid using up storage. Instead it only captures data on activy to caputre user intent.
Any user activity triggers a capture where Dystil the quicky traverses the AXtree / UIA and stores relevant nodes. This is then used to extract text visible on screen (and semantic meaning capture plane for release in future)
Then deterministic redaction runs on this data even before its stored locally.

Asyncronously a worker goes throug this data and creatss a work_index to make it eassier and token efficient for LLMs and your AI agnest to retrieve data.
Another worker runs ML-PII on the data for ensure absolute PII redaction.

A third worker runs local embedding model on work_index to allow for semantic search down the line.

MCP allows for bounded search over users work so that AI agentss only get relevant information.
