# Reading a text into a library

A reader turns bytes into two things: the text a tid is taken over, and a table of nodes over
that text. Nothing else in the library interprets a file format.

## What a node is

A node is a range of the text with an address. The range is authoritative and the address is
informative, which is the only arrangement that survives a reader finding a structure its
source states wrongly.

- a verse is a node
- a paragraph is a node
- a line is a node
