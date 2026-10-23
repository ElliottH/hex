#include "arch/parakeet/phrase-boost.h"
#include <cassert>

int main() {
    using transcribe::parakeet::PhraseBoost;
    PhraseBoost boost;
    boost.nodes.resize(3);
    boost.nodes[0].edges[1] = 1; // word-boundary first piece
    boost.nodes[1].edges[2] = 2; // continuation, never a root candidate
    boost.depth = 2;
    boost.bonus = 1;
    boost.margin = .5;
    int node = 0;
    const float start[] = {-10, -.3, -5, 0, -8};
    assert(boost.choose(start, 5, 4, node) == 1 && node == 1);
    const float continuation[] = {-10, -5, -.2, 0, -8};
    assert(boost.choose(continuation, 5, 4, node) == 2 && node == 2);
    node = 0;
    assert(boost.choose(continuation, 5, 4, node) == 3 && node == 0);
    const float confident[] = {-10, -.6, -5, 0, -8};
    assert(boost.choose(confident, 5, 4, node) == 3);
    const float distant[] = {-.1, -2, -5, 0, -8};
    assert(boost.choose(distant, 5, 4, node) == 3); // small top-two gap isn't enough
    const float blank[] = {-10, -.2, -5, -4, 0};
    node = 1;
    assert(boost.choose(blank, 5, 4, node) == 4 && node == 1);
    boost.bonus = 0;
    node = 0;
    assert(boost.choose(start, 5, 4, node) == 3);
    // A streaming chunk can reconstruct the same cursor from its bounded
    // emitted-token suffix, including after unrelated text.
    assert(boost.advance(boost.advance(0, 3), 1) == 1);
    assert(boost.advance(1, 3) == 0);
}
