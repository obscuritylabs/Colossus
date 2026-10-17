#include "policy_transition.h"
#include <stdlib.h>
#include <string.h>

#define POLICY_LIMIT (128U*1024U)
struct parser { const char* source;size_t size,position,length;char* output; };
static int append(struct parser* parser,const char* bytes,size_t size) {
  if(size>POLICY_LIMIT-parser->length)return 0;
  memcpy(parser->output+parser->length,bytes,size);parser->length+=size;return 1;
}
static int whitespace(char value) { return value==' '||value=='\t'||value=='\n'||value=='\r'; }
static void skip(struct parser* parser) {
  while(parser->position<parser->size) {
    char value=parser->source[parser->position];
    if(whitespace(value)) {++parser->position;continue;}
    if(value==';') { while(parser->position<parser->size&&parser->source[parser->position]!='\n')++parser->position;continue; }
    break;
  }
}
static int token(struct parser* parser,size_t* begin,size_t* size) {
  skip(parser);*begin=parser->position;
  while(parser->position<parser->size) {
    char value=parser->source[parser->position];
    if(whitespace(value)||value=='('||value==')'||value==';'||value=='"')break;
    ++parser->position;
  }
  *size=parser->position-*begin;return *size>0;
}
static int equal(struct parser* parser,size_t begin,size_t size,const char* value) {
  return strlen(value)==size&&!memcmp(parser->source+begin,value,size);
}
static int process(struct parser* parser,size_t begin,size_t size) {
  static const char* operations[]={"process","process*","process-fork","process-exec","process-exec*","process-exec-interpreter","process-exec-update-label","job-creation","mach-register"};
  for(size_t i=0;i<sizeof(operations)/sizeof(operations[0]);++i)if(equal(parser,begin,size,operations[i]))return 1;
  return 0;
}
static int expression(struct parser* parser,unsigned depth) {
  if(depth>64||parser->position>=parser->size||parser->source[parser->position]!='(')return 0;
  const size_t begin=parser->position++;const size_t out_begin=parser->length;
  size_t operator_begin=0,operator_size=0;
  /* Scheme let/let* binding lists have an expression as their first element.
   * They are not empty lists or an alternate operation selector. */
  if(!token(parser,&operator_begin,&operator_size)&&
     (parser->position>=parser->size||parser->source[parser->position]!='('))return 0;
  int remove=0;
  if(equal(parser,operator_begin,operator_size,"allow")) {
    const size_t saved=parser->position;unsigned process_count=0,other_count=0;
    while(1) {skip(parser);if(parser->position==parser->size)return 0;
      char value=parser->source[parser->position];if(value=='('||value==')')break;
      size_t operation_begin=0,operation_size=0;if(!token(parser,&operation_begin,&operation_size))return 0;
      if(process(parser,operation_begin,operation_size))++process_count;else ++other_count;
    }
    if(process_count&&other_count)return 0;
    remove=process_count>0;parser->position=saved;
  }
  if(!append(parser,parser->source+begin,parser->position-begin))return 0;
  while(parser->position<parser->size) {
    size_t segment=parser->position;
    if(parser->source[parser->position]=='(') {if(!expression(parser,depth+1))return 0;continue;}
    if(parser->source[parser->position]==')') {
      ++parser->position;if(!append(parser,parser->source+segment,1))return 0;
      if(remove) {parser->length=out_begin;return append(parser,"#f",2);} return 1;
    }
    if(parser->source[parser->position]=='"') {
      ++parser->position;int ended=0;
      while(parser->position<parser->size) {char value=parser->source[parser->position++];
        if(value=='\\') {if(parser->position==parser->size)return 0;++parser->position;}
        else if(value=='"') {ended=1;break;}
      }
      if(!ended)return 0;
    } else if(parser->source[parser->position]==';') {
      while(parser->position<parser->size&&parser->source[parser->position]!='\n')++parser->position;
    } else ++parser->position;
    if(!append(parser,parser->source+segment,parser->position-segment))return 0;
  }
  return 0;
}
int colossus_owned_process_policy(const char* pinned,size_t size,char** output) {
  if(!output)return 0;*output=NULL;
  if(!pinned||!size||size>POLICY_LIMIT||memchr(pinned,0,size))return 0;
  struct parser parser={pinned,size,0,0,malloc(POLICY_LIMIT+1)};
  if(!parser.output)return 0;
  while(parser.position<size) {
    size_t before=parser.position;skip(&parser);
    if(!append(&parser,pinned+before,parser.position-before))goto fail;
    if(parser.position==size)break;
    if(size-parser.position>=2&&!memcmp(pinned+parser.position,"#f",2)&&
       (parser.position+2==size||whitespace(pinned[parser.position+2])||pinned[parser.position+2]==';')) {
      if(!append(&parser,pinned+parser.position,2))goto fail;parser.position+=2;continue;
    }
    if(!expression(&parser,0))goto fail;
  }
  static const char final[]="\n; Final immutable creator boundary: no positive exec/fork grant survives.\n"
    "(deny process-fork process-exec*)\n"
    "(deny job-creation mach-register)\n"
    "(deny mach-priv-task-port mach-task-name mach-task-read mach-task-inspect)\n"
    "(deny signal)\n(allow signal (target self))\n";
  if(!append(&parser,final,sizeof(final)-1))goto fail;
  parser.output[parser.length]=0;*output=parser.output;return 1;
fail:free(parser.output);return 0;
}
