import { createHash } from 'node:crypto';

export type GitKind = 'blob' | 'tree' | 'commit';

/** A git object: its id, and its bytes with and without the header. */
export interface GitObject {
  id: string;
  kind: GitKind;
  body: Buffer;
  /** `<kind> <size>\0<body>`, uncompressed. */
  headed: Buffer;
}

export function gitObject(kind: GitKind, body: Buffer | string): GitObject {
  const content = Buffer.from(body);
  const headed = Buffer.concat([
    Buffer.from(`${kind} ${content.length}\0`),
    content,
  ]);
  return {
    id: createHash('sha1').update(headed).digest('hex'),
    kind,
    body: content,
    headed,
  };
}

/** A tree of `[mode, name, object]` entries. */
export function treeOf(entries: [string, string, GitObject][]): GitObject {
  return gitObject(
    'tree',
    Buffer.concat(
      entries.map(([mode, name, object]) =>
        Buffer.concat([
          Buffer.from(`${mode} ${name}\0`),
          Buffer.from(object.id, 'hex'),
        ])
      )
    )
  );
}

export function commitOf(tree: GitObject, message: string): GitObject {
  const person = 'Ada <ada@example.test> 1700000000 +0000';
  return gitObject(
    'commit',
    `tree ${tree.id}\nauthor ${person}\ncommitter ${person}\n\n${message}\n`
  );
}
