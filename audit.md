1. architectural: further refine structure to reduce complexity. current file layout is a bit complicated.
2. reporter:
    a. enforce one program, one reporter policy, no matter how many working subprocesses or threads reports to it.
    b. probably combine scope (resource limit) here? or somewhere else in workflow. Downstream user projects should not bother with custom code for scope settings.
    c. cosmatic: add color to status flag. allow adding horizontal splitter line by specifying label number before it. this better split tasks into phases if needed.
    d. tasks that do not have steps, simply do something once and update status, should be handled slightly differently? maybe split the display into a progress section and a message section?
3. state and series: implement eq and partial eq?
