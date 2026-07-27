-- Application validation gives a useful error, while this index closes the
-- race where two provider tasks try to finish the same request concurrently.
CREATE UNIQUE INDEX agent_messages_one_terminal_reply
ON agent_messages (in_reply_to)
WHERE kind IN ('response', 'error');
