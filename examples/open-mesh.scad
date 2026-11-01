// A polyhedron with one face missing. Tests the open-edge warning and STEP
// export of an open shell (OPEN_SHELL instead of a solid).

size = 20; // [5:1:60]
missing_face = true;

faces = [
    [0, 1, 2, 3], // bottom
    [4, 5, 1, 0], [7, 6, 5, 4], [5, 6, 2, 1], [6, 7, 3, 2], [7, 4, 0, 3],
];
polyhedron(
    points = [[0, 0, 0], [size, 0, 0], [size, size, 0], [0, size, 0],
              [0, 0, size], [size, 0, size], [size, size, size], [0, size, size]],
    faces = missing_face ? [for (i = [1 : len(faces) - 1]) faces[i]] : faces
);
